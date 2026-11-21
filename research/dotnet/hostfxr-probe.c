/*
 * T1 probe (docs/06_dotnet_bridge.md): load the CoreCLR that ships inside the
 * pinned pwsh into this native process through hostfxr, call managed static
 * methods through function pointers, and report what each step costs.
 *
 *     hostfxr-probe MODE PWSH_DIR PROBE_DLL
 *
 * MODE
 *   cmdline        hostfxr_initialize_for_dotnet_command_line with pwsh.dll as
 *                  the application, then hdt_load_assembly_and_get_function_pointer
 *                  (the probe library goes into an isolated load context)
 *   runtimeconfig  hostfxr_initialize_for_runtime_config with
 *                  pwsh.runtimeconfig.json (expected to fail: the runtime is
 *                  self-contained)
 *   default        as cmdline, then hdt_load_assembly + hdt_get_function_pointer
 *                  into the default load context (.NET 8 API)
 *   named          as default, with a named coreclr_load_context instead of the
 *                  default context (.NET 11 API in dotnet/runtime main)
 *
 * PROBE_APP=<path.dll> names the application instead of PWSH_DIR/pwsh.dll, and
 * PROBE_SKIP_SMA=1 skips the steps that need System.Management.Automation.
 *
 * Every step prints one line:
 *   step <name> rc=<hex> at_ms=<since main> delta_ms=<since previous> rss_kib=<VmRSS> hwm_kib=<VmHWM>
 *
 * Nothing in the pinned pwsh directory is written; the application is only
 * named, never run.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

typedef void *hostfxr_handle;

struct hostfxr_initialize_parameters {
    size_t size;
    const char *host_path;
    const char *dotnet_root;
};

enum hostfxr_delegate_type {
    hdt_com_activation,
    hdt_load_in_memory_assembly,
    hdt_winrt_activation,
    hdt_com_register,
    hdt_com_unregister,
    hdt_load_assembly_and_get_function_pointer,
    hdt_get_function_pointer,
    hdt_load_assembly,
    hdt_load_assembly_bytes,
};

typedef int32_t (*init_command_line_fn)(int argc, const char **argv,
                                        const struct hostfxr_initialize_parameters *parameters,
                                        hostfxr_handle *handle);
typedef int32_t (*init_runtime_config_fn)(const char *runtime_config_path,
                                          const struct hostfxr_initialize_parameters *parameters,
                                          hostfxr_handle *handle);
typedef int32_t (*get_runtime_delegate_fn)(const hostfxr_handle handle, enum hostfxr_delegate_type type,
                                           void **delegate);
typedef int32_t (*close_fn)(const hostfxr_handle handle);

#define UNMANAGEDCALLERSONLY_METHOD ((const char *)-1)

struct coreclr_load_context {
    size_t size;
    const char *identifier;
};

typedef int (*load_assembly_and_get_function_pointer_fn)(const char *assembly_path, const char *type_name,
                                                         const char *method_name,
                                                         const char *delegate_type_name, void *reserved,
                                                         void **delegate);
typedef int (*get_function_pointer_fn)(const char *type_name, const char *method_name,
                                       const char *delegate_type_name, void *load_context, void *reserved,
                                       void **delegate);
typedef int (*load_assembly_fn)(const char *assembly_path, void *load_context, void *reserved);

typedef int (*add_fn)(int, int);
typedef int (*read_length_fn)(const char *);
typedef int (*no_args_fn)(void);

static struct timespec started, previous;

static double since(const struct timespec *from, const struct timespec *to)
{
    return (to->tv_sec - from->tv_sec) * 1e3 + (to->tv_nsec - from->tv_nsec) / 1e6;
}

static long status_kib(const char *field)
{
    FILE *status = fopen("/proc/self/status", "r");
    char line[256];
    long value = -1;
    size_t length = strlen(field);
    if (status == NULL)
        return -1;
    while (fgets(line, sizeof line, status)) {
        if (strncmp(line, field, length) == 0 && line[length] == ':') {
            value = strtol(line + length + 1, NULL, 10);
            break;
        }
    }
    fclose(status);
    return value;
}

static void step(const char *name, long rc)
{
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    printf("step %-28s rc=%#lx at_ms=%.3f delta_ms=%.3f rss_kib=%ld hwm_kib=%ld\n", name,
           (unsigned long)rc, since(&started, &now), since(&previous, &now), status_kib("VmRSS"),
           status_kib("VmHWM"));
    fflush(stdout);
    previous = now;
}

static void *symbol(void *library, const char *name)
{
    void *address = dlsym(library, name);
    if (address == NULL) {
        fprintf(stderr, "hostfxr-probe: %s not found: %s\n", name, dlerror());
        exit(2);
    }
    return address;
}

int main(int argc, char **argv)
{
    clock_gettime(CLOCK_MONOTONIC, &started);
    previous = started;
    if (argc != 4) {
        fprintf(stderr, "usage: hostfxr-probe cmdline|runtimeconfig|default|named PWSH_DIR PROBE_DLL\n");
        return 2;
    }
    const char *mode = argv[1];
    const char *pwsh_dir = argv[2];
    const char *probe_dll = argv[3];
    char path[PATH_MAX], app[PATH_MAX], config[PATH_MAX], self[PATH_MAX];
    ssize_t self_length = readlink("/proc/self/exe", self, sizeof self - 1);
    if (self_length < 0)
        return 2;
    self[self_length] = '\0';
    snprintf(path, sizeof path, "%s/libhostfxr.so", pwsh_dir);
    if (getenv("PROBE_APP") != NULL)
        snprintf(app, sizeof app, "%s", getenv("PROBE_APP"));
    else
        snprintf(app, sizeof app, "%s/pwsh.dll", pwsh_dir);
    snprintf(config, sizeof config, "%s/pwsh.runtimeconfig.json", pwsh_dir);
    step("start", 0);

    void *hostfxr = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    if (hostfxr == NULL) {
        fprintf(stderr, "hostfxr-probe: %s\n", dlerror());
        return 2;
    }
    init_command_line_fn init_command_line = symbol(hostfxr, "hostfxr_initialize_for_dotnet_command_line");
    init_runtime_config_fn init_runtime_config = symbol(hostfxr, "hostfxr_initialize_for_runtime_config");
    get_runtime_delegate_fn get_runtime_delegate = symbol(hostfxr, "hostfxr_get_runtime_delegate");
    close_fn close_context = symbol(hostfxr, "hostfxr_close");
    step("dlopen hostfxr", 0);

    struct hostfxr_initialize_parameters parameters = {sizeof parameters, self, pwsh_dir};
    hostfxr_handle context = NULL;
    int32_t rc;
    if (strcmp(mode, "runtimeconfig") == 0) {
        rc = init_runtime_config(config, &parameters, &context);
        step("initialize (runtimeconfig)", rc);
        if (rc < 0 || context == NULL)
            return 1;
    } else {
        const char *command_line[] = {app};
        rc = init_command_line(1, command_line, &parameters, &context);
        step("initialize (command line)", rc);
        if (rc < 0 || context == NULL)
            return 1;
    }

    add_fn add = NULL;
    read_length_fn read_length = NULL;
    no_args_fn regex_matches = NULL, touch_sma = NULL, run_powershell = NULL;

    if (strcmp(mode, "named") == 0 || strcmp(mode, "default") == 0) {
        load_assembly_fn load_assembly = NULL;
        get_function_pointer_fn get_function_pointer = NULL;
        rc = get_runtime_delegate(context, hdt_load_assembly, (void **)&load_assembly);
        step("delegate hdt_load_assembly", rc);
        if (rc < 0)
            return 1;
        rc = get_runtime_delegate(context, hdt_get_function_pointer, (void **)&get_function_pointer);
        step("delegate get_function_ptr", rc);
        if (rc < 0)
            return 1;
        struct coreclr_load_context named = {sizeof named, "probe-module"};
        void *module = strcmp(mode, "named") == 0 ? (void *)&named : NULL;
        rc = load_assembly(probe_dll, module, NULL);
        step(module ? "load into named context" : "load into default context", rc);
        if (rc < 0)
            return 1;
        rc = get_function_pointer("Probe, probe", "Add", UNMANAGEDCALLERSONLY_METHOD, module, NULL,
                                  (void **)&add);
        step("function pointer Add", rc);
        if (rc < 0)
            return 1;
        rc = get_function_pointer("Probe, probe", "ReadLength", UNMANAGEDCALLERSONLY_METHOD, module,
                                  NULL, (void **)&read_length);
        rc |= get_function_pointer("Probe, probe", "RegexMatches", UNMANAGEDCALLERSONLY_METHOD, module,
                                   NULL, (void **)&regex_matches);
        rc |= get_function_pointer("Probe, probe", "TouchSma", UNMANAGEDCALLERSONLY_METHOD, module, NULL,
                                   (void **)&touch_sma);
        rc |= get_function_pointer("Probe, probe", "RunPowerShell", UNMANAGEDCALLERSONLY_METHOD, module,
                                   NULL, (void **)&run_powershell);
        step("function pointers (rest)", rc);
        if (rc < 0)
            return 1;
    } else {
        load_assembly_and_get_function_pointer_fn load = NULL;
        rc = get_runtime_delegate(context, hdt_load_assembly_and_get_function_pointer, (void **)&load);
        step("delegate (starts runtime)", rc);
        if (rc < 0 || load == NULL)
            return 1;
        rc = load(probe_dll, "Probe, probe", "Add", UNMANAGEDCALLERSONLY_METHOD, NULL, (void **)&add);
        step("load assembly + Add", rc);
        if (rc < 0)
            return 1;
        rc = load(probe_dll, "Probe, probe", "ReadLength", UNMANAGEDCALLERSONLY_METHOD, NULL,
                  (void **)&read_length);
        rc |= load(probe_dll, "Probe, probe", "RegexMatches", UNMANAGEDCALLERSONLY_METHOD, NULL,
                   (void **)&regex_matches);
        rc |= load(probe_dll, "Probe, probe", "TouchSma", UNMANAGEDCALLERSONLY_METHOD, NULL,
                   (void **)&touch_sma);
        rc |= load(probe_dll, "Probe, probe", "RunPowerShell", UNMANAGEDCALLERSONLY_METHOD, NULL,
                   (void **)&run_powershell);
        step("function pointers (rest)", rc);
        if (rc < 0)
            return 1;
    }

    int sum = add(2, 3);
    step("first call Add", sum);
    sum = add(sum, 4);
    step("second call Add", sum);
    {
        /* The cost of one native-to-managed call once everything is warm. */
        struct timespec before, after;
        int value = 0;
        clock_gettime(CLOCK_MONOTONIC, &before);
        for (int i = 0; i < 1000000; i++)
            value = add(value, 1);
        clock_gettime(CLOCK_MONOTONIC, &after);
        /* Milliseconds for a million calls are nanoseconds per call. */
        printf("calls Add 1000000 ns_per_call=%.2f result=%d\n", since(&before, &after), value);
        step("1e6 calls Add", value);
    }
    int length = read_length(config);
    step("File.ReadAllText", length);
    length = read_length(config);
    step("File.ReadAllText again", length);
    step("Regex.Matches", regex_matches());
    if (getenv("PROBE_SKIP_SMA") == NULL) {
        step("touch SMA", touch_sma());
        step("PowerShell.Create + 1+1", run_powershell());
        step("again PowerShell + 1+1", run_powershell());
    }

    close_context(context);
    step("close", 0);
    return 0;
}

/*
 * Runs a command and reports its wall time and resource usage:
 *
 *     spawn-measure FD PROGRAM [ARG...]
 *     spawn-measure --path FILE PROGRAM [ARG...]
 *     -> "<wall ns> <user us> <sys us> <max rss KiB> <minor faults> <major faults> <wait status>\n"
 *
 * scripts/time-commands.py passes a file descriptor; `cargo harness bench`
 * passes a report path, so no descriptor has to survive the exec.
 *
 * Linux charges a process's peak RSS from before exec to the program it
 * execs, so a child forked from a Python process of 20 MiB reports at least
 * 20 MiB, however small it is. Forked from this program instead, it reports
 * its own.
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

int main(int argc, char **argv)
{
    FILE *report;
    int program;
    if (argc > 1 && strcmp(argv[1], "--path") == 0) {
        if (argc < 4) {
            fprintf(stderr, "usage: spawn-measure --path FILE PROGRAM [ARG...]\n");
            return 2;
        }
        report = fopen(argv[2], "w");
        if (report == NULL) {
            perror("spawn-measure: report file");
            return 2;
        }
        program = 3;
    } else {
        if (argc < 3) {
            fprintf(stderr, "usage: spawn-measure FD PROGRAM [ARG...]\n");
            return 2;
        }
        int fd = atoi(argv[1]);
        report = fdopen(fd, "w");
        if (report == NULL) {
            perror("spawn-measure: report descriptor");
            return 2;
        }
        program = 2;
    }
    fcntl(fileno(report), F_SETFD, FD_CLOEXEC);

    struct timespec start, end;
    clock_gettime(CLOCK_MONOTONIC, &start);
    pid_t pid = fork();
    if (pid < 0) {
        perror("spawn-measure: fork");
        return 2;
    }
    if (pid == 0) {
        execv(argv[program], argv + program);
        perror("spawn-measure: exec");
        _exit(127);
    }
    int status;
    struct rusage usage;
    if (wait4(pid, &status, 0, &usage) < 0) {
        perror("spawn-measure: wait4");
        return 2;
    }
    clock_gettime(CLOCK_MONOTONIC, &end);

    long long wall = (long long)(end.tv_sec - start.tv_sec) * 1000000000LL
                   + (end.tv_nsec - start.tv_nsec);
    fprintf(report, "%lld %lld %lld %ld %ld %ld %d\n", wall,
            (long long)usage.ru_utime.tv_sec * 1000000 + usage.ru_utime.tv_usec,
            (long long)usage.ru_stime.tv_sec * 1000000 + usage.ru_stime.tv_usec,
            usage.ru_maxrss, usage.ru_minflt, usage.ru_majflt, status);
    fclose(report);
    return 0;
}

# The .NET bridge

How rpsh answers a script that names a .NET type, and what each answer
costs. Settles #6; DESIGN.md §5 summarises it.

Measurements: `bench/baselines/2026-11-21-ryzen5-4650u-dotnet-t1.json`
(`research/dotnet/measure.py` over `research/dotnet/hostfxr-probe.c`) and
`bench/baselines/2026-11-21-ryzen5-4650u-dotnet-ipc.json`
(`cargo bench -p rpsh-research --bench dotnet_ipc`).

## The three tiers

| Tier | What runs | First use | Later |
|---|---|---|---|
| T0 | the type or member is in rpsh's native shim | nothing | nothing |
| T1 | CoreCLR is loaded into the rpsh process through `hostfxr` | 29 ms before the first managed call | a direct function pointer or a cached delegate |
| T2 | a per-user sidecar process keeps CoreCLR and assemblies loaded | 6 µs per request after the sidecar is warm | 6 µs per request |

A script's tier is computed from its AST before it runs (#131), so a
benchmark can report "this script is T1 because of `[System.IO.File]`" and a
user can ask why a start is slow.

## T0: what the shim covers

Scripts name .NET types directly. `research/dotnet/type-literals.py` counts
type literals in the `.ps1` and `.psm1` files shipped with pwsh 7.7.0, as a
stand-in until the corpus analysis of #18. It found 1,388 literals over 123
distinct names in 144 files. The most frequent, with static accesses
(`X::member`) in the second column:

| type | uses | static |
|---|---:|---:|
| `string` | 382 | 11 |
| `string[]` | 101 | 0 |
| `switch` | 90 | 0 |
| `ref` | 78 | 0 |
| `Microsoft.PowerShell.PSConsoleReadLine` | 63 | 63 |
| `uri` | 61 | 4 |
| `StringComparison` | 49 | 49 |
| `bool` | 33 | 0 |
| `pscredential` | 33 | 0 |
| `Net.ServicePointManager` | 30 | 30 |
| `IO.Path` | 26 | 26 |
| `ExitCode` | 25 | 25 |
| `version` | 24 | 5 |
| `ordered` | 19 | 0 |
| `Management.Automation.ErrorCategory` | 19 | 15 |
| `pscustomobject` | 18 | 0 |
| `int` | 15 | 3 |
| `Management.Automation.PSCmdlet` | 13 | 0 |
| `psresource` | 13 | 12 |
| `object[]` | 12 | 0 |
| `double` | 11 | 0 |
| `Net.SecurityProtocolType` | 10 | 10 |
| `guid` | 10 | 5 |
| `datetime` | 10 | 6 |
| `type` | 10 | 0 |
| `scope` | 10 | 0 |

The corpus is biased: it is the modules pwsh ships, so it over-represents
`PSReadLine`, `PSResource` and `PSModuleInfo` and under-represents what
scripts in the wild do. It still shows the shape of the shim:

* the primitive and near-primitive types (`String`, `Char`, `Math`,
  `Convert`, `Array`, `Enum`, `Guid`, `Version`, `Uri`, `Random`,
  `Environment`, `Console`) and the collection types carry most uses and
  are #49 / #52;
* `Path`, `File` and `Directory` are the first I/O surface scripts reach,
  #51;
* `StringBuilder`, `Encoding` and the regex API are next, #50;
* `Net`, `Security`, `Threading`, `Diagnostics` and `Xml` are small in
  count but each is a whole shim area, #53;
* the `System.Management.Automation` types scripts see (`PSCmdlet`,
  `ErrorCategory`, `PSObject`, `PSModuleInfo`) are #54; binary modules see
  the same public surface, below.

The shim grows by measured use, not by transcribing the BCL. A type that is
not in it goes to T1.

## T1: loading the runtime in-process

`research/dotnet/build.sh` builds `hostfxr-probe.c` and a managed
`Probe.cs`; `measure.py` runs each mode 11 times and keeps the median of
every step. The probe loads the `libhostfxr.so` that ships **inside the
pinned pwsh**, starts the runtime from the command line, loads the probe
into a load context, resolves unmanaged-callable function pointers and calls
them. Nothing in the pwsh directory is written.

Default mode, median cumulative milliseconds and RSS:

| step | at | RSS MiB |
|---|---:|---:|
| start | 0.06 | 2.1 |
| `dlopen libhostfxr.so` | 2.25 | 4.4 |
| `hostfxr_initialize_for_dotnet_command_line` | 3.31 | 5.2 |
| get the `load_assembly` delegate (starts CoreCLR) | 20.74 | 18.2 |
| get the `get_function_pointer` delegate | 20.83 | 18.2 |
| load the probe into the default load context | 26.57 | 27.7 |
| resolve the function pointers | 30.42 | 29.0 |
| first call into managed code | 30.89 | 29.0 |
| 1,000,000 calls | 37.88 | 29.0 |
| `File.ReadAllText` | 39.64 | 29.4 |
| `Regex.Matches` | 47.16 | 32.8 |
| touch `System.Management.Automation` | 48.80 | 36.8 |
| `PowerShell.Create()` + `1+1` | 192.90 | 84.2 |
| a second `PowerShell` + `1+1` | 213.77 | 85.8 |

What it says:

* **`dlopen` is free; starting CoreCLR is not.** 2.2 ms to open hostfxr,
  17.4 ms more to start the runtime. From an empty process, the first
  managed call costs 31 ms.
* **After that, calls are cheap.** A million calls through a function
  pointer take 7.25 ms, or 7.3 ns each. `File.ReadAllText` is 2.0 ms the
  first time (JIT and framework initialization) and 120 µs the second.
* **SMA is a second program.** Loading and running PowerShell in the same
  runtime costs another 145 ms and 55 MiB. The bridge must not reach for
  SMA to answer a `[System.IO.Path]` call.
* **Two initialization modes were rejected by the pinned runtime.**
  `hostfxr_initialize_for_runtime_config` fails with "Initialization for
  self-contained components is not supported" because pwsh is published
  self-contained; the named load-context API (`coreclr_load_context`)
  returned `0x80131502` for a load into a named context. Both are re-tested
  when rpsh ships its own `.runtimeconfig.json`; today the command-line
  initialization above is the supported path.

T1's own process is what a script pays once. It does not pay it twice for
the same assembly, and it does not pay it at all when the script stays in
T0.

### Runtime discovery

In order, and only when a script needs T1:

1. `DOTNET_ROOT` with a compatible `libhostfxr.so`;
2. an installed `pwsh` (the same discovery the harness uses);
3. a runtime bundled with rpsh (a cargo feature, off by default).

If none is found, the run stops with `rpsh: not implemented: .NET runtime
for [System.X]` and exit code 99, never an approximation. T0 scripts are
unaffected.

## T2: the warm sidecar

`dotnet_ipc` measures the transport alone: length-prefixed frames over a
Unix domain socket, the server echoing each frame. Median round trip:

| payload | thread in-process | separate process | connect + round trip |
|---|---:|---:|---:|
| 64 B | 6.34 µs | 6.50 µs | 13.23 µs |
| 4 KiB | 8.38 µs | 9.35 µs | — |
| 64 KiB | 24.94 µs | 25.75 µs | — |

The transport is not the cost; starting the runtime is. A sidecar pays the
31 ms once per user and then a shell's repeated invocations pay microseconds
instead. It is worth it exactly when many rpsh processes need .NET:
interactive shells, `tool init | Invoke-Expression`, CI matrices.

Design:

* one sidecar per user, a Unix socket under `$XDG_RUNTIME_DIR` (or
  `$TMPDIR`), started on demand by the first T1-needing process and idle-
  exited after a timeout;
* frames of `u32` little-endian length plus payload; a request names a type
  and member, handles are client-scoped integers, and the protocol carries
  a version so that an old sidecar is replaced rather than misread;
* the first request after the sidecar starts waits for CoreCLR; later ones
  do not. A failed sidecar falls back to T1 in the same process, so a broken
  daemon degrades to a slower correct answer, never a wrong one;
* it is off unless enabled, like the bundled runtime. #130 owns the
  implementation.

## CLR objects at the boundary

* A CLR object is a handle, not a copy: an `i64` index into a table owned by
  the bridge, with a generation to catch stale handles. `[object]::ReferenceEquals`
  on two handles to the same object is true, and a script that passes an
  object back gets the same one.
* Member access is a cached delegate per (type, member, signature). A miss
  resolves through reflection once and stores the delegate; argument
  conversions go through the same `LanguagePrimitives.ConvertTo` rules as
  the native runtime (#126).
* Values that cross the boundary keep their identity in both directions.
  An rpsh string passed to a T1 method is a .NET `string` at that moment; a
  .NET `string` returned to rpsh is stored by the value model of #35.
* Exceptions become `ErrorRecord`s with the .NET exception as the target,
  and the stack trace is kept as a note property.

## Binary modules and the SMA facade

`research/dotnet/sma-api.ps1` reports the public surface of
`System.Management.Automation` 7.7.0: **829 exported types**, of which 674
classes, 114 enums, 30 interfaces, 7 structs and 4 delegates, and **9,020
public members**, of which 1,570 are ordinary methods. 64 exported classes
derive from `Cmdlet`.

`research/dotnet/sma-references.ps1` reads the metadata of every DLL in
pwsh's bundled modules (PSReadLine, ThreadJob, PSResourceGet,
PackageManagement) without loading them. The union is **169 SMA types and
335 members**, in five namespaces:

| namespace | types |
|---|---:|
| `System.Management.Automation` | 119 |
| `System.Management.Automation.Language` | 35 |
| `System.Management.Automation.Runspaces` | 12 |
| `Microsoft.PowerShell.Commands` | 2 |
| `System.Management.Automation.Host` | 1 |

So a facade assembly can be useful long before it is complete: 20% of the
types and 4% of the members cover everything pwsh itself ships. #128 owns
the facade, starting from this union and growing by the modules scripts
actually load.

`research/dotnet/bundled-modules.ps1` also shows why module isolation
matters: PSResourceGet ships 47 assemblies including its own
`Newtonsoft.Json`, `System.Text.Json` and NuGet stack, and PackageManagement
ships a second provider set. Each module needs its dependencies resolved
independently (#127). The probe's named-context API is the right mechanism;
until it works on the pinned runtime, the fallback is the default load
context plus a version check on the facade.

## Prior art

None of the surveyed shells embeds or hosts a managed runtime. fish, brush,
yash-rs and ion execute external programs (fish 4.9.3 (`6cf3137`),
`src/exec.rs:86`; brush `brush-shell-v0.4.0` (`96a26d0`),
`brush-core/src/interp.rs:186`; yash-rs `yash-cli-3.6.0` (`46d2f79`),
`yash-env/src/system/real.rs:932`; ion `master` (`e9b3ca4`),
`src/lib/shell/pipe_exec/mod.rs:502`), and Nushell's extension mechanism is
its own plugin protocol, not a CLR bridge. T1 and T2 have no direct prior art
here; none of these shells solves the CLR problem.

**T2 lifecycle: Nushell plugins.** Plugin commands are declared from cached
signatures in a brotli-compressed MessagePack registry without spawning the
plugin (Nushell 0.117.0 (`a481925`),
`crates/nu-plugin-engine/src/init.rs:269-286`;
`crates/nu-protocol/src/plugin/registry_file/mod.rs:61-76, 90-94`). Plugin GC
stops idle plugin processes
(`crates/nu-protocol/src/config/plugin_gc.rs:5-6`,
`crates/nu-plugin-engine/src/gc.rs:11-15`). *Inference:* registry-without-spawn
plus idle reaping is the nearest lifecycle precedent for T2; the length-
prefixed Unix-domain socket is rps's own choice, not Nushell's protocol.

**T0 and pipes: Nushell `ByteStream`.** `PipelineData::ByteStream`
(`crates/nu-protocol/src/pipeline/pipeline_data.rs:49-55`) and `into_stdio`
(`crates/nu-protocol/src/pipeline/byte_stream.rs:560-587`, used at
`crates/nu-command/src/system/run_external.rs:256-261`) hand a child's pipe
directly to the next with no decoding — a contrast for native pipes only,
since the pipe carries bytes between processes, not CLR objects.

Takeaway. T0's scope follows corpus frequency, as above. T1 does what none of
these shells does, so 31 ms to the first managed call and 7.3 ns per call
after is the whole argument. T2's nearest lifecycle precedent is Nushell's
registry plus idle reaping; the socket is rps's own. Because the four shells
only spawn external commands, the 6.5 µs warm-sidecar tradeoff is rps-specific.

## What the implementation issues take from this

| Issue | Scope fixed here |
|---|---|
| #49–#54 | T0 shim areas, in the order above; each grows by measured use |
| #125 | T1: runtime discovery, `hostfxr` initialization, handles, delegates |
| #126 | T1: value conversion between `Value` and CLR objects |
| #127 | T1: per-module `AssemblyLoadContext`, lazy dependency resolution |
| #128 | SMA facade assembly, scoped by the metadata scan above |
| #129 | `Add-Type`: compile and load in the T1 runtime, no C# compiler in rpsh |
| #130 | T2 sidecar: protocol, lifecycle, fallback |
| #131 | tier analysis from the AST, reported by the harness |

Rejected:

* **Make everything T1.** A script that asks for `[Math]::Max` would pay 29
  ms and 29 MiB. The shim exists to make the common answer free.
* **A separate .NET process per call (T2 without a warm sidecar).** Connect
  plus round trip is 13 µs, but a cold process pays the full 29 ms; only a
  resident sidecar amortizes it.
* **Compile C# in rpsh.** `Add-Type` is a T1 operation: the C# compiler is
  not part of the base runtime, and pwsh ships Roslyn separately. rpsh
  invokes a compiler only when a script uses `Add-Type` (#129).

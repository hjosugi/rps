# rpsh

`rpsh` is an implementation of PowerShell in Rust. The command is also
`rpsh`. It is measured against the latest stable upstream
[PowerShell](https://github.com/PowerShell/PowerShell) release: the goal is to
run `.ps1` scripts the way `pwsh` does, cover what `pwsh` ships, and run Linux
commands the way a shell should.

<div class="grid cards" markdown>

-   **Measured against pwsh**

    Output and exit codes are compared with recorded `pwsh` runs. Anything
    unsupported is a refusal, never an approximation.

    [How conformance works](usage/compatibility.md)

-   **No .NET runtime**

    A native Rust engine, value model, pipeline and file system. The
    architecture is deliberately small so startup stays cheap.

    [Architecture](project/architecture.md)

-   **Runs scripts**

    Variables, control flow, functions, classes, pipelines, common cmdlets,
    modules and native commands. The surface grows by conformance case.

    [Status](reference/status.md)

-   **Built in the open**

    Every behaviour change starts with a differential test; every release is
    a reviewable commit and a version bump.

    [Contributing](project/contributing.md)

</div>

## Try it

```console
$ cargo build --release
$ target/release/rpsh -Command "(1 + 2) * 3"
9
$ target/release/rpsh -Command "'alpha', 'beta' | ForEach-Object { $_.ToUpper() }"
ALPHA
BETA
```

A script runs the same way:

```powershell
# hello.ps1
param([string] $Name = 'world')
"Hello, $Name"
```

```console
$ target/release/rpsh -File hello.ps1 -Name rpsh
Hello, rpsh
```

## What it is not

- It is not a wrapper around `pwsh`: there is no .NET runtime in the process.
- It is not a subset with "close enough" output. If a construct is not
  implemented, `rpsh` exits with code 99 and names what is missing.
- It is not finished. Coverage is tracked case by case in the conformance
  suite and in the inventory.

## Next steps

- [Install](get-started/install.md)
- [Quick start](get-started/quickstart.md)
- [Command line](usage/cli.md)
- [Compatibility](usage/compatibility.md)

# Command line

The command-line parser follows upstream's `CommandLineParameterParser.cs`.

```text
Usage: rpsh [-NoProfile] [-NoLogo] [-NonInteractive] [[-File] <filePath>]
            [-Command <string>] [-Version] [-Help]
```

## Invocations

| Form | Behaviour |
|---|---|
| `rpsh -Command <text>` | Runs the rest of the command line, joined with spaces. |
| `rpsh -File <path> [args...]` | Runs a UTF-8 `.ps1` file; remaining arguments become `$args` or bind to `param()`. |
| `rpsh <path> [args...]` | The first non-switch argument is treated as `-File`. |
| `rpsh -Version` | Prints the version and the PowerShell release it targets. |
| `rpsh -Help` | Prints usage. |

`-NoProfile`, `-NoLogo` and `-NonInteractive` are accepted for compatibility.
`-NoProfile` is the effective behaviour: there is no profile loading yet.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | The command or script completed and `exit` was not given another value. |
| `99` | `rpsh` does not implement something the input used. Stderr begins with `rpsh: not implemented:`. |
| `64` | The command line was invalid (for example `-Command` with no text). |
| other | The value passed to `exit`. |

A script's `exit N` becomes the process exit code. Native commands set
`$LASTEXITCODE` but do not terminate the script unless it chooses to.

## Environment

The session environment is available through `$env:NAME`. Assigning
`$env:NAME` (or removing it with `$null`) affects native commands started
later in the same session. `$HOME` is used for `~` expansion by path cmdlets.

## Not implemented yet

These are refusals, not silent fallbacks:

- interactive sessions (`rpsh` with no command);
- `-Command -` and `-File -` (script from standard input);
- `-EncodedCommand` and the other less common switches.

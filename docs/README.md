# Documentation index

[DESIGN.md](../DESIGN.md) is normative: the architecture, and the numbered
sections that issues cite. The documents here record how its open decisions
were settled: what was measured, what was chosen, and what was rejected and
why. Numbers in them cite a file under [bench/baselines](../bench/baselines).

Numeric prefixes match the M0 issue that settled the decision.

| Document | Decision | Issue |
|---|---|---|
| [01_compat_contract.md](01_compat_contract.md) | what "the same as the reference pwsh" means and how it is enforced | #1 |
| [02_value_model.md](02_value_model.md) | how values are represented; compact strings, 16-byte `Value` | #2 |
| [03_memory.md](03_memory.md) | reference counting with a backup cycle collector | #3 |
| [04_execution.md](04_execution.md) | a register IR with inline caches, interpreted | #4 |
| [05_startup.md](05_startup.md) | where pwsh's 156 ms goes and rpsh's 5 ms budget | #5 |
| [06_dotnet_bridge.md](06_dotnet_bridge.md) | T0 shim, T1 `hostfxr`, T2 sidecar; the SMA facade | #6 |
| [07_dependencies.md](07_dependencies.md) | what may be linked, and how a crate is admitted | #7 |
| [07_pester_plan.md](07_pester_plan.md) | how the upstream Pester suite is classified and scored | #16 |

A numeric result in these documents names the PowerShell version it was
measured against. `scripts/fetch-pwsh.sh` installs the latest stable release;
`tests/conformance/reference.txt` records the version the current goldens use.

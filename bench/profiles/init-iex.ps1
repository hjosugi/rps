# The "tool init | Invoke-Expression" shape: run a native tool, parse its
# output as PowerShell and run it.
& "$PSScriptRoot/stub-tool.sh" | Invoke-Expression

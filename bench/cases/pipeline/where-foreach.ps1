# The object pipeline: one script block invocation per element per stage.
1..100000 | Where-Object { $_ % 3 -eq 0 } | ForEach-Object { $_ * 2 } | Measure-Object -Sum | ForEach-Object Sum

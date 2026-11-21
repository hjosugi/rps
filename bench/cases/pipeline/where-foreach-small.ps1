# The object pipeline at 1/10 the size of where-foreach.
1..10000 | Where-Object { $_ % 3 -eq 0 } | ForEach-Object { $_ * 2 } | Measure-Object -Sum | ForEach-Object Sum

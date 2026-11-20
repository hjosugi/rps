1..10 | Where-Object { $_ % 2 -eq 0 } | ForEach-Object { $_ * $_ }
1..5 | Measure-Object -Sum | ForEach-Object Sum
'b', 'a', 'c' | Sort-Object
1..10 | Select-Object -First 3
1..10 | Select-Object -Skip 8

# ForEach-Object: one script block invocation per pipeline element.
$sum = 0
1..100000 | ForEach-Object { $sum += $_ }
$sum

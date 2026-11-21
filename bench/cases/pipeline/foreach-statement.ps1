# foreach statement over the same range: no pipeline machinery.
$sum = 0
foreach ($n in 1..100000) { $sum += $n }
$sum

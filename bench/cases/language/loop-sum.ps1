# A counted loop: variable access, comparison, increment, addition.
$sum = 0
for ($i = 0; $i -lt 1000000; $i++) { $sum += $i }
$sum

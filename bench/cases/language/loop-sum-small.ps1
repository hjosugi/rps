# A short counted loop, paired with loop-sum to show the size effect.
$sum = 0
for ($i = 0; $i -lt 100000; $i++) { $sum += $i }
$sum

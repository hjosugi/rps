# Measure-Object over 100k pipeline objects.
$sum = (1..100000 | Measure-Object -Sum).Sum
$sum

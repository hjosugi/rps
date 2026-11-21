# Select-Object -First should stop the upstream pipeline early.
$first = 1..200000 | Select-Object -First 100
$first.Count
$first[-1]

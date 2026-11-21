# Read a large native command's output line by line.
$count = 0
seq 1 200000 | ForEach-Object { $count++ }
$count

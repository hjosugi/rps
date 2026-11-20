# Bytes between native commands, and native output read line by line.
seq 1 200000 | wc -l
(seq 1 50000 | Measure-Object -Line).Lines
seq 1 50000 | ForEach-Object { $_.Length } | Measure-Object -Sum | ForEach-Object Sum

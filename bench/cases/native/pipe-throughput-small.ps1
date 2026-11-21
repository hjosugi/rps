# A smaller byte stream between native commands.
seq 1 20000 | wc -l
(seq 1 5000 | Measure-Object -Line).Lines

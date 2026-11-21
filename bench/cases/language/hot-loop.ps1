# rpsh-bench: runs=3 warmup=1
# A long arithmetic loop: the region where upstream's JIT is expected to win.
$sum = 0
for ($i = 0; $i -lt 1500000; $i++) { $sum += $i }
$sum

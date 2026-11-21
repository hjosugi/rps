# Throwing and catching exceptions in a loop, at 5x the small case.
$count = 0
for ($i = 0; $i -lt 5000; $i++) {
    try { throw "failure $i" } catch { $count++ }
}
$count

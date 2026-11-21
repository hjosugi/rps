# Throwing and catching exceptions in a loop, at a small size.
$count = 0
for ($i = 0; $i -lt 1000; $i++) {
    try { throw "failure $i" } catch { $count++ }
}
$count

# Select-String over a stream of 20k strings.
$lines = foreach ($i in 1..20000) { "line $i status=$($i % 7)" }
$hits = $lines | Select-String -Pattern 'status=0' -SimpleMatch
$hits.Count

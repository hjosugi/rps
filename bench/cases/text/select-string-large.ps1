# Select-String over a stream of 100k strings.
$lines = foreach ($i in 1..100000) { "line $i status=$($i % 7)" }
$hits = $lines | Select-String -Pattern 'status=0' -SimpleMatch
$hits.Count

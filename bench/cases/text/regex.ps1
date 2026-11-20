# -match and -replace over generated lines.
$hits = 0
$replaced = 0
foreach ($i in 1..50000) {
    $line = "2026-11-20 host$($i % 50) GET /api/items/$i 200 $($i % 997)ms"
    if ($line -match 'host(\d+) GET (\S+) (\d{3})') { $hits += [int]$Matches[1] }
    $replaced += ($line -replace '\d+ms$', 'Xms').Length
}
$hits
$replaced

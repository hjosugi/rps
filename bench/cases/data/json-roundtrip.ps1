# ConvertTo-Json and ConvertFrom-Json over a few thousand objects.
$rows = foreach ($i in 1..5000) {
    [ordered]@{ id = $i; name = "row$i"; tags = @('a', 'b', 'c'); nested = [ordered]@{ ok = ($i % 2 -eq 0); score = $i * 1.5 } }
}
$json = $rows | ConvertTo-Json -Depth 5 -Compress
$json.Length
$back = $json | ConvertFrom-Json
$back.Count
($back | Measure-Object -Property id -Sum).Sum
$back[4999].nested.score

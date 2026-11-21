# ConvertTo-Csv and ConvertFrom-Csv over a small table.
$rows = foreach ($i in 1..1000) {
    [pscustomobject]@{ id = $i; name = "row$i"; score = $i % 100 }
}
$csv = $rows | ConvertTo-Csv -NoTypeInformation
$csv.Count
$back = $csv | ConvertFrom-Csv
$back.Count
($back | Measure-Object -Property id -Sum).Sum

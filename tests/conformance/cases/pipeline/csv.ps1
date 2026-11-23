# rpsh-conformance: level=c2
$rows = [pscustomobject]@{ a = 1; b = 'x' }, [pscustomobject]@{ a = 2; b = 'y' }
$csv = $rows | ConvertTo-Csv
$csv -join ';'
($csv | ConvertFrom-Csv | ForEach-Object { "$($_.a)$($_.b)" }) -join ','

# rpsh-conformance: level=c2
if (Test-Path rpsh-rows.csv) { Remove-Item rpsh-rows.csv }
$rows = [pscustomobject]@{ a = 1; b = 'x' }, [pscustomobject]@{ a = 2; b = 'y' }
$rows | Export-Csv rpsh-rows.csv
(Get-Content rpsh-rows.csv) -join ';'
(Import-Csv rpsh-rows.csv | ForEach-Object { "$($_.a)$($_.b)" }) -join ','
Remove-Item rpsh-rows.csv

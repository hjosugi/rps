# rpsh-conformance: level=c2
if (Test-Path rpsh-rows.csv) { Remove-Item rpsh-rows.csv }
$rows = [pscustomobject]@{ a = 1; b = 'x' }, [pscustomobject]@{ a = 2; b = 'y' }
$rows | Export-Csv rpsh-rows.csv
(Get-Content rpsh-rows.csv) -join ';'
(Import-Csv rpsh-rows.csv | ForEach-Object { "$($_.a)$($_.b)" }) -join ','
Remove-Item rpsh-rows.csv
'1,2', '3,4' | ConvertFrom-Csv -Header A, B | ForEach-Object { "$($_.A)$($_.B)" }
@('x,y', '1,2') | ConvertFrom-Csv | ForEach-Object { "$($_.x)$($_.y)" }

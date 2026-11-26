# rpsh-conformance: level=c2
if (Test-Path rpsh-rows.csv) { Remove-Item rpsh-rows.csv }
$rows = [pscustomobject]@{ a = 1; b = 'x' }, [pscustomobject]@{ a = 2; b = 'y' }
$rows | Export-Csv rpsh-rows.csv
(Get-Content rpsh-rows.csv) -join ';'
(Import-Csv rpsh-rows.csv | ForEach-Object { "$($_.a)$($_.b)" }) -join ','
Remove-Item rpsh-rows.csv
'1,2', '3,4' | ConvertFrom-Csv -Header A, B | ForEach-Object { "$($_.A)$($_.B)" }
@('x,y', '1,2') | ConvertFrom-Csv | ForEach-Object { "$($_.x)$($_.y)" }
[pscustomobject]@{a=1;b='x'} | ConvertTo-Csv -NoHeader
[pscustomobject]@{a=1;b='x'} | ConvertTo-Csv
[pscustomobject]@{a=1;b='x'} | Export-Csv rpsh-nh.csv -NoHeader
(Get-Content rpsh-nh.csv) -join ';'
Remove-Item rpsh-nh.csv
[pscustomobject]@{a=1;b='x'} | ConvertTo-Csv -Delimiter ';'
[pscustomobject]@{a=1;b='x'} | Export-Csv rpsh-semi.csv -Delimiter ';'
(Get-Content rpsh-semi.csv) -join '|'
Remove-Item rpsh-semi.csv
'1;2', '3;4' | ConvertFrom-Csv -Delimiter ';' -Header A, B | ForEach-Object { "$($_.A)$($_.B)" }
@('a;b', '1;2') | ConvertFrom-Csv -Delimiter ';' | ForEach-Object { "$($_.a)$($_.b)" }
[pscustomobject]@{a=1} | Export-Csv rpsh-append.csv
[pscustomobject]@{a=2} | Export-Csv rpsh-append.csv -Append
(Get-Content rpsh-append.csv) -join '|'
Remove-Item rpsh-append.csv
[pscustomobject]@{a=1} | Export-Csv rpsh-append2.csv -Append
(Get-Content rpsh-append2.csv) -join '|'
Remove-Item rpsh-append2.csv
Set-Content rpsh-import-a.csv -Value @('a,b', '1,2')
Set-Content rpsh-import-b.csv -Value @('a,b', '3,4')
(Import-Csv -Path rpsh-import-a.csv, rpsh-import-b.csv | ForEach-Object { "$($_.a)$($_.b)" }) -join ','
Remove-Item rpsh-import-a.csv
Remove-Item rpsh-import-b.csv

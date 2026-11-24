# rpsh-conformance: level=c2
Get-Content fixtures/data.txt -Raw
Get-Content fixtures/data.txt -Tail 1
Get-Content fixtures/data.txt -First 1
Get-Content fixtures/data.txt -Last 1
'abc' | Out-File rpsh-out-nonl.txt -NoNewline
(Get-Content rpsh-out-nonl.txt -Raw).Length
'abc' | Out-File rpsh-out-nl.txt
(Get-Content rpsh-out-nl.txt -Raw).Length
Remove-Item rpsh-out-nonl.txt
Remove-Item rpsh-out-nl.txt
Set-Content rpsh-delim.txt -Value 'a,b,' -NoNewline
(Get-Content rpsh-delim.txt -Delimiter ',' | ForEach-Object { "[$_]" }) -join ';'
(Get-Content rpsh-delim.txt -Delimiter ',').Count
Remove-Item rpsh-delim.txt
Get-Content fixtures/data.txt -TotalCount 1
Set-Content rpsh-values.txt 'a', 'b'
(Get-Content rpsh-values.txt) -join ','
Set-Content rpsh-values2.txt -Value 'c', 'd'
(Get-Content rpsh-values2.txt) -join ','
Add-Content rpsh-values.txt 'e', 'f'
(Get-Content rpsh-values.txt) -join ','
Remove-Item rpsh-values.txt
Remove-Item rpsh-values2.txt
(Get-Content fixtures/tree/*.txt) -join ','
(Get-Content fixtures/data.txt, fixtures/tree/a.txt) -join ','
(Get-Content fixtures/data.txt -Raw).Length

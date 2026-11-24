# rpsh-conformance: level=c2
if (Test-Path rpsh-tee.txt) { Remove-Item rpsh-tee.txt }
if (Test-Path rpsh-no-nl.txt) { Remove-Item rpsh-no-nl.txt }
'c', 'a', 'b', 'a' | Sort-Object -Unique
1, 2, 3 | Tee-Object -FilePath rpsh-tee.txt | ForEach-Object { $_ * 2 }
(Get-Content rpsh-tee.txt) -join ','
Set-Content rpsh-no-nl.txt -Value 'abc' -NoNewline
Get-Content rpsh-no-nl.txt -Raw
2, 3 | Tee-Object -FilePath rpsh-tee.txt -Append | Out-Null
(Get-Content rpsh-tee.txt) -join ','
Remove-Item rpsh-tee.txt
Remove-Item rpsh-no-nl.txt
1, 2 | Tee-Object -Variable x | Out-Null
$x -join ','
$x.Count

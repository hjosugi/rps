# rpsh-conformance: level=c2
Write-Output 1 2 3
Write-Output 'a', 'b'
Write-Output (1..3) -NoEnumerate | ForEach-Object { $_.Count }
echo 'via alias'

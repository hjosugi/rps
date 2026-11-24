# rpsh-conformance: level=c2
Write-Output 1 2 3
Write-Output 'a', 'b'
Write-Output (1..3) -NoEnumerate | ForEach-Object { $_.Count }
echo 'via alias'
1, 2 | Out-Host
'x' | Out-Default
Write-Host a b
Write-Host 'a', 'b'
Write-Host 'x'
Write-Host a b -Separator '-'
Write-Host 'a' 'b' -Separator ''
Write-Verbose 'hidden'
Write-Debug 'dbg'
Write-Output 'visible'

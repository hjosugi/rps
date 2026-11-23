# rpsh-conformance: level=c2
if (Test-Path rpsh-ac.txt) { Remove-Item rpsh-ac.txt }
Write-Output -InputObject 1
Write-Output -InputObject (1, 2)
Add-Content -Path rpsh-ac.txt -Value 'a' -NoNewline
Add-Content -Path rpsh-ac.txt -Value 'b' -NoNewline
Get-Content rpsh-ac.txt -Raw
Remove-Item rpsh-ac.txt

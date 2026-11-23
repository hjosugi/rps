# rpsh-conformance: level=c2
if (Test-Path rpsh-cg-dst) { Remove-Item rpsh-cg-dst -Recurse -Force }
Set-Content rpsh-cg1.tmp -Value '1'
Set-Content rpsh-cg2.tmp -Value '2'
$null = New-Item -ItemType Directory -Path rpsh-cg-dst -Force
Copy-Item rpsh-cg*.tmp rpsh-cg-dst
(Get-ChildItem rpsh-cg-dst -Name | Sort-Object) -join ','
Remove-Item rpsh-cg-dst -Recurse -Force
Remove-Item rpsh-cg1.tmp
Remove-Item rpsh-cg2.tmp

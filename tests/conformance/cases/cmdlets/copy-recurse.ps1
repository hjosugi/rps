# rpsh-conformance: level=c2
if (Test-Path rpsh-src) { Remove-Item rpsh-src -Recurse -Force }
if (Test-Path rpsh-dst) { Remove-Item rpsh-dst -Recurse -Force }
$null = New-Item -ItemType Directory -Path rpsh-src -Force
Set-Content rpsh-src/a.txt -Value 'x'
$null = Copy-Item rpsh-src rpsh-dst -Recurse
Test-Path rpsh-dst/a.txt
Get-Content rpsh-dst/a.txt
$null = New-Item -ItemType File -Path rpsh-v.txt -Value 'hello'
Get-Content rpsh-v.txt
Remove-Item rpsh-src -Recurse -Force
Remove-Item rpsh-dst -Recurse -Force
Remove-Item rpsh-v.txt

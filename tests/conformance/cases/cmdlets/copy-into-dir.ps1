# rpsh-conformance: level=c2
if (Test-Path rpsh-cd) { Remove-Item rpsh-cd -Recurse -Force }
if (Test-Path rpsh-src.txt) { Remove-Item rpsh-src.txt }
$null = New-Item -ItemType Directory -Path rpsh-cd -Force
Set-Content rpsh-src.txt -Value 'x'
Copy-Item rpsh-src.txt rpsh-cd
Get-Content rpsh-cd/rpsh-src.txt
Move-Item rpsh-cd/rpsh-src.txt rpsh-cd/moved.txt
Test-Path rpsh-cd/moved.txt
Remove-Item rpsh-cd -Recurse -Force
Remove-Item rpsh-src.txt

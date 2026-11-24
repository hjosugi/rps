# rpsh-conformance: level=c2
if (Test-Path rpsh-files-test) { Remove-Item rpsh-files-test -Recurse -Force }
$null = New-Item -ItemType Directory -Path rpsh-files-test -Force
Set-Content rpsh-files-test/a.txt -Value 'one'
Add-Content rpsh-files-test/a.txt -Value 'two'
Get-Content rpsh-files-test/a.txt
Copy-Item rpsh-files-test/a.txt rpsh-files-test/b.txt
Test-Path rpsh-files-test/b.txt
Get-ChildItem rpsh-files-test | ForEach-Object Name
Remove-Item rpsh-files-test/b.txt
Test-Path rpsh-files-test/b.txt
Rename-Item rpsh-files-test/a.txt -NewName c.txt
(Get-ChildItem rpsh-files-test | ForEach-Object Name) -join ','
Remove-Item rpsh-files-test -Recurse -Force
Test-Path rpsh-files-test
Set-Content rpsh-del1.txt -Value 'x'
Set-Content rpsh-del2.txt -Value 'y'
Remove-Item rpsh-del1.txt, rpsh-del2.txt
Test-Path rpsh-del1.txt
Test-Path rpsh-del2.txt

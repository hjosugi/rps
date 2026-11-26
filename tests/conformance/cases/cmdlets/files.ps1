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
if (Test-Path rpsh-copy-many) { Remove-Item rpsh-copy-many -Recurse -Force }
$null = New-Item -ItemType Directory -Path rpsh-copy-many -Force
Set-Content rpsh-copy-a.txt -Value 'a'
Set-Content rpsh-copy-b.txt -Value 'b'
Copy-Item rpsh-copy-a.txt, rpsh-copy-b.txt -Destination rpsh-copy-many
(Get-ChildItem rpsh-copy-many -Name | Sort-Object) -join ','
Remove-Item rpsh-copy-many -Recurse -Force
Remove-Item rpsh-copy-a.txt
Remove-Item rpsh-copy-b.txt
if (Test-Path rpsh-move-many) { Remove-Item rpsh-move-many -Recurse -Force }
$null = New-Item -ItemType Directory -Path rpsh-move-many -Force
Set-Content rpsh-move-a.txt -Value 'a'
Set-Content rpsh-move-b.txt -Value 'b'
Move-Item rpsh-move-a.txt, rpsh-move-b.txt -Destination rpsh-move-many
(Get-ChildItem rpsh-move-many -Name | Sort-Object) -join ','
Test-Path rpsh-move-a.txt
Remove-Item rpsh-move-many -Recurse -Force
Set-Content rpsh-ro.txt -Value 'x'
chmod 444 rpsh-ro.txt
(Get-ChildItem rpsh-ro.txt -Attributes ReadOnly -Name) -join ','
(Get-ChildItem rpsh-ro.txt -Attributes '!ReadOnly' -Name) -join ','
chmod 644 rpsh-ro.txt
Remove-Item rpsh-ro.txt
Set-Content rpsh-prop.txt -Value 'x'
Set-ItemProperty -Path rpsh-prop.txt -Name IsReadOnly -Value $true
(Get-ItemProperty -Path rpsh-prop.txt).IsReadOnly
Set-ItemProperty -Path rpsh-prop.txt -Name IsReadOnly -Value $false
(Get-ItemProperty -Path rpsh-prop.txt).IsReadOnly
Remove-Item rpsh-prop.txt
if (Test-Path rpsh-move-glob) { Remove-Item rpsh-move-glob -Recurse -Force }
$null = New-Item -ItemType Directory -Path rpsh-move-glob -Force
Set-Content rpsh-glob1.txt -Value 'a'
Set-Content rpsh-glob2.txt -Value 'b'
Move-Item rpsh-glob*.txt -Destination rpsh-move-glob
(Get-ChildItem rpsh-move-glob -Name | Sort-Object) -join ','
Test-Path rpsh-glob1.txt
Remove-Item rpsh-move-glob -Recurse -Force
if (Test-Path rpsh-link-test) { Remove-Item rpsh-link-test -Recurse -Force }
$null = New-Item -ItemType Directory -Path rpsh-link-test -Force
Set-Content rpsh-link-test/target.txt -Value 'hello'
New-Item -ItemType SymbolicLink -Path rpsh-link-test/link.txt -Target target.txt | Out-Null
(Get-Item rpsh-link-test/link.txt).LinkType
(Get-Item rpsh-link-test/link.txt).Target
Test-Path rpsh-link-test/link.txt
Remove-Item rpsh-link-test -Recurse -Force
Test-Path rpsh-link-test

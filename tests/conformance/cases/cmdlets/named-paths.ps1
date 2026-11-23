# rpsh-conformance: level=c2
if (Test-Path rpsh-np) { Remove-Item rpsh-np -Recurse -Force }
$null = New-Item -ItemType Directory -Path rpsh-np -Force
$null = New-Item -ItemType File -Path rpsh-np -Name a.txt -Value 'hi'
(Get-Item -Path rpsh-np/a.txt).Name
$null = Copy-Item -Path rpsh-np/a.txt -Destination rpsh-np/b.txt
Get-Content rpsh-np/b.txt
Move-Item -Path rpsh-np/b.txt -Destination rpsh-np/c.txt
Test-Path rpsh-np/c.txt
Remove-Item rpsh-np -Recurse -Force

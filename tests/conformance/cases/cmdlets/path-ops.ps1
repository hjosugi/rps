# rpsh-conformance: level=c2
Test-Path fixtures/data.txt -PathType Leaf
Test-Path fixtures -PathType Container
Test-Path fixtures/data.txt -PathType Container
(Resolve-Path fixtures/data.txt).Path -eq (Join-Path (Get-Location).Path 'fixtures/data.txt')
Test-Path fixtures/*.txt
Test-Path fixtures/*.nope
Split-Path fixtures/data.txt -Extension
Split-Path fixtures/data.txt -LeafBase
Test-Path fixtures/tree/*.txt -PathType Leaf
Test-Path fixtures/tree/*.txt -PathType Container
Test-Path -LiteralPath fixtures/data.txt
Test-Path -Path fixtures/data.txt
(Get-Item -LiteralPath fixtures/data.txt).Name
Split-Path /tmp/x -IsAbsolute
Split-Path fixtures/data.txt -IsAbsolute
(Get-Content -LiteralPath fixtures/data.txt) -join ','
Test-Path 'foo' -IsValid
Test-Path '' -IsValid
Test-Path 'a/b' -IsValid
Test-Path 'a:b' -IsValid
Test-Path -IsValid 'foo'
(Resolve-Path fixtures/tree/*.txt | ForEach-Object { Split-Path $_.Path -Leaf }) -join ','
Test-Path '~'
Test-Path '~/'
Test-Path '~/definitely-not-here'
(Get-Item '~').PSIsContainer
(Get-ChildItem '~' -Directory | Measure-Object).Count -gt 0
Test-Path fixtures/data.txt, fixtures/nope
Join-Path -Path fixtures -ChildPath data.txt
Join-Path -Path fixtures -ChildPath tree -AdditionalChildPath a.txt
Join-Path fixtures data.txt
Split-Path -Path /tmp/x -Leaf
Split-Path -Path fixtures/data.txt -Parent
Split-Path -Path /tmp/a.txt -Extension
$env:RPSH_PROVIDER_TEST = 'v'
Test-Path Env:RPSH_PROVIDER_TEST
Test-Path Env:RPSH_PROVIDER_NOPE
$provider = 1
Test-Path Variable:provider
Test-Path Variable:nope
Test-Path Alias:gc
$env:RPSH_CHILD_A = '1'
$env:RPSH_CHILD_B = '2'
(Get-ChildItem Env: | Where-Object Name -like 'RPSH_CHILD_*' | ForEach-Object Name | Sort-Object) -join ','
$childVar = 1
(Get-ChildItem Variable:child* | ForEach-Object Name | Sort-Object) -join ','
Set-Alias localalias Write-Output
(Get-ChildItem Alias:localalias -Name) -join ','
function LocalChild {}
(Get-ChildItem Function:LocalChild -Name) -join ','
$env:RPSH_REMOVE = 'v'
Remove-Item Env:RPSH_REMOVE
Test-Path Env:RPSH_REMOVE
$rv = 1
Remove-Item Variable:rv
Test-Path Variable:rv
Set-Alias localrm Write-Output
Remove-Item Alias:localrm
Test-Path Alias:localrm
function LocalRm {}
Remove-Item Function:LocalRm
Test-Path Function:LocalRm
Test-Path Env:
Test-Path Variable:
Test-Path Alias:
Test-Path Function:
Set-Item -Path Env:RPSH_SET -Value 'v'
$env:RPSH_SET
Set-Item -Path Variable:sv -Value 1
$sv
Test-Path Env:RPSH_SET
Remove-Item Env:RPSH_SET
Remove-Variable sv
New-Item -Path Env:RPSH_NEW -Value 'v' | Out-Null
$env:RPSH_NEW
Test-Path Env:RPSH_NEW
New-Item -Path Variable:nv -Value 2 | Out-Null
$nv
Test-Path Variable:nv
Remove-Item Env:RPSH_NEW
Remove-Variable nv
$x = New-Item -Path Env:RPSH_ASSIGN -Value 'v'
$x.Name
$env:RPSH_ASSIGN
Remove-Item Env:RPSH_ASSIGN
New-Item -Path Env: -Name RPSH_N -Value 'v' | Out-Null
$env:RPSH_N
New-Item -Path Variable: -Name pv -Value 3 | Out-Null
$pv
Test-Path Env:RPSH_N
Test-Path Variable:pv
Remove-Item Env:RPSH_N
Remove-Variable pv
Split-Path -Path /tmp/x -NoQualifier
Split-Path -Path fixtures/data.txt -NoQualifier

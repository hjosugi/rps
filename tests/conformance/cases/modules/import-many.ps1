# rpsh-conformance: level=c2
Import-Module ./module-fixture.psm1, ./module-extra.psm1
ModuleFn
Get-Extra 2
(Get-Command -Module module-extra | ForEach-Object Name) -join ','
(Get-Command -Module module-fixture | ForEach-Object Name | Sort-Object) -join ','
(Get-Command -Module module-fixture, module-extra | ForEach-Object Name | Sort-Object) -join ','
(Get-Module -Name module-* | ForEach-Object Name | Sort-Object) -join ','
(Get-Module -Name module-fixture, module-extra | ForEach-Object Name | Sort-Object) -join ','
Remove-Module module-*
Import-Module ./module-fixture.psm1, ./module-extra.psm1
Remove-Module module-fixture, module-extra
(Get-Module -Name module-* | Measure-Object).Count
Test-Path Function:ModuleFn
Test-Path Function:Get-Extra
Import-Module ./module-extra.*
Get-Extra 1
(Get-Module -Name module-extra | Measure-Object).Count
Remove-Module module-extra

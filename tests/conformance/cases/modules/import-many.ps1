# rpsh-conformance: level=c2
Import-Module ./module-fixture.psm1, ./module-extra.psm1
ModuleFn
Get-Extra 2
(Get-Module -Name module-* | ForEach-Object Name | Sort-Object) -join ','
Remove-Module module-*

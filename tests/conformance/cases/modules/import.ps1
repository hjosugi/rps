# rpsh-conformance: level=c2
Import-Module ./module-fixture.psm1
ModuleFn
Add-One 4
(Get-Module -Name module-fixture).Name
(Get-Module -Name module-fixture).ModuleType
(Test-Path Function:ModuleFn)
Remove-Module -Name module-fixture
(Get-Module -Name module-fixture | Measure-Object).Count
Test-Path Function:ModuleFn

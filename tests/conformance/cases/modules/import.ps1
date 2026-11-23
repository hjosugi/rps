# rpsh-conformance: level=c2
Import-Module ./module-fixture.psm1
ModuleFn
Add-One 4
(Get-Module -Name module-fixture).Name
(Get-Module -Name module-fixture).ModuleType

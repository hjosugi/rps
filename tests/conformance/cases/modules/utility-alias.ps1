# rpsh-conformance: level=c2
$utility = 'Microsoft.PowerShell.Utility'
(Get-Module $utility).Count
Test-Path Alias:fhx
$available = Get-Command -Name 'fh?' -CommandType Alias
"$($available.Name)|$($available.ModuleName)|$($null -eq $available.Definition)"
(Get-Command -Name 'fh?' -ListImported).Count
Get-Item Alias:gcb, Alias:gtz, Alias:scb | ForEach-Object {
    "$($_.Name)|$($_.Definition)|$($_.ModuleName)|$($_.Options)"
}
function Write-Output { 'local function' }
Write-Output
(Get-Module $utility).Count
Remove-Item Function:Write-Output
$found = Get-Command -Name FhX
"$($found.Name)|$($found.Definition)|$($found.ModuleName)|$($found.Source)"
$found.Version.ToString()
$found.Version.Major
$module = Get-Module $utility
"$($module.Name)|$($module.ModuleType)|$($module.Version)"
(Get-Alias).Count
Remove-Item Alias:fhx
Test-Path Alias:fhx
(Get-Alias -Name 'fh*').Count
Import-Module Microsoft.PowerShell.Utility
(Get-Item Alias:fhx).Definition
Set-Alias fhx Get-Item
Remove-Module $utility
(Get-Module $utility).Count
(Get-Item Alias:fhx).Definition
Import-Module $utility
(Get-Item Alias:fhx).ModuleName
Remove-Module $utility
Test-Path Alias:fhx
$PSModuleAutoLoadingPreference = 'None'
(Get-Command -Name fhx -ErrorAction SilentlyContinue).Count
Test-Path Alias:fhx
$PSModuleAutoLoadingPreference = 'All'
Write-Output 'load Utility'
Test-Path Alias:fhx

# rpsh-conformance: level=c2
(Get-Alias -Name echo).Definition
(Get-Alias -Name gc).Definition
Set-Alias greet Write-Output
greet 'hello'
(Get-Alias -Definition Write-Output | ForEach-Object Name) -join ','
(Get-Alias -Definition Get-Content | ForEach-Object Name) -join ','
(Get-Alias -Name echo, gc | ForEach-Object Name | Sort-Object) -join ','
(Get-Alias echo, gc | ForEach-Object Name | Sort-Object) -join ','
(Get-Alias -Definition Write-Output, Get-Content | ForEach-Object Name | Sort-Object) -join ','

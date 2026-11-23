# rpsh-conformance: level=c2
(Get-Alias -Name echo).Definition
(Get-Alias -Name gc).Definition
Set-Alias greet Write-Output
greet 'hello'

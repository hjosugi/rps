# rpsh-conformance: level=c2
# Get-Alias itself imports Utility and includes its exported alias.
Get-Alias
'Out-String'
Get-Alias type, gc | Out-String
'Narrow'
Get-Alias echo | Out-String -Width 20
'Wide'
Get-Alias echo | Out-String -Width 120
'AutoSize'
Get-Alias echo, md | Format-Table -AutoSize | Out-String
'Hidden headers'
Get-Alias echo | Format-Table -HideTableHeaders | Out-String
'Wide characters'
Set-Alias 測試 Write-Output
Get-Alias 測試 | Out-String -Width 24
'Command discovery'
$alias = Get-Command type
$alias.GetType().FullName
"$alias"
$alias | Out-String
'Native names'
foreach ($name in 'ac','ls','cp','mv','rm','cat','ps','kill','sort','tee','sleep','man','clear','diff') {
    Test-Path "Alias:$name"
}

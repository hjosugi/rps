# rpsh-conformance: level=c2
# Module imports can add aliases. Check the session's own definitions.
Get-Alias | Where-Object { -not $_.ModuleName } | ForEach-Object {
    "$($_.Name)|$($_.Definition)|$($_.Options)"
}
(Get-Item Alias:GM).Options
(Get-ChildItem Alias:sel*).Options
(Get-Command gerr).CommandType
"one", "two" | measure | ForEach-Object Count

Set-Alias tYpE Write-Output
(Get-Alias type).Name
(Get-Alias type).Definition
(Get-Command type | Measure-Object).Count
type 'overridden'
Remove-Item Alias:type
Test-Path Alias:type
Set-Alias gAl Write-Output -Force
(Get-Item Alias:gal).Options
gAl 'forced'
Remove-Item Alias:gAl
Test-Path Alias:gal

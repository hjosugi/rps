# rpsh-conformance: level=c2
# Whole strings (including leading, trailing and inter-set newlines) and record boundaries.
(Get-Command -Syntax sort-object)
(Get-Command -Name Switch-Process -Syntax)
(Get-Command Sort-Object).Definition -ceq (Get-Command Sort-Object -Syntax)
(Get-Command -Module Microsoft.PowerShell.Utility -Name Sort-Object -Syntax)
(Get-Command -Module Microsoft.PowerShell.Core -Name Sort-Object,Switch-Process -Syntax)
@(Get-Command -Syntax Sort-Object,Sort-Object).Count
foreach ($line in (Get-Command -Syntax Switch-Process,Sort-Object)) { $line }
Set-Alias -Name ListSorted -Value Sort-Object
Set-Alias -Name AnotherList -Value ListSorted
(Get-Command listSORTED -Syntax)
(Get-Command AnotherList -Syntax)
(Get-Command Microsoft.PowerShell.Utility\Sort-Object -Syntax)
(Get-Command -Name 'Switch-Proce*' -Syntax)
(Get-Command -Syntax:$false Sort-Object).Name
Set-Alias -Name SendRecord -Value Write-Output
(Get-Command SendRecord -Syntax)
(Get-Command Write-Output).Definition -ceq (Get-Command Write-Output -Syntax)
Write-Output -InputObject $null | ForEach-Object { 'bound null' }
$empty = Write-Output -InputObject @() -NoEnumerate
"$($empty.GetType().FullName) $($empty.Count)"

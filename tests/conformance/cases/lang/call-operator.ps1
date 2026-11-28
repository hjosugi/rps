# rpsh-conformance: level=c2 LANG=C
$name='Write-Output'
& $name -InputObject 'space in argument'
& ('Write-'+'Output') 'joined name'
$names='Write-Output','Get-Date'
& $names[0] -InputObject 42
$command=[pscustomobject]@{Name='Write-Output'}
& $command.Name -InputObject 'member'
$items='one','two words'
& "Write-Output" @items

$outer='parent'
$block={param($label) $outer='child'; "$label|$args|$outer"; return 'done'}
& $block -label bound extra
$outer
1, 2, 3 | & { "input=$input" }
$result = & {param($x) $x+1} 6
$result

function Show-Tag($a) { "tag:$a" }
$functionInfo=Get-Command Show-Tag
Set-Alias -Name TagAlias -Value Show-Tag
$aliasInfo=Get-Alias TagAlias
Set-Alias -Name TagAlias -Value Get-Date
& $functionInfo function
& $aliasInfo alias
$cmdletInfo=Get-Command Write-Output
function Write-Output($a) { "shadow:$a" }
& 'Write-Output' normal
& $cmdletInfo original
foreach ($invalid in @($null,'',@())) {
    try { & $invalid } catch { $_.FullyQualifiedErrorId; $_.Exception.GetType().Name }
}

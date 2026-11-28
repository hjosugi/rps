# rpsh-conformance: level=c2
(Get-Command Write-Output).Definition
(Get-Command echo -Syntax)
$a = Write-Output -NoEnumerate 7
"$($a.GetType().Name):$($a.Capacity):$($a.Count):$($a[0])"
$a = Write-Output 7 8 9 10 11 -No
"$($a.GetType().Name):$($a.Capacity):$($a -join ',')"
$a = Write-Output -No -InputObject 7
$a.GetType().Name
$l = [Collections.ArrayList]@(1,2)
$a = Write-Output -No $l
$a.Equals($l)
$a = Write-Output -No 'one','two'
"$($a.GetType().FullName) $($a.Count)"
Write-Output -No:$false (1,2) (3,4) | ForEach-Object { "$($_.GetType().Name) [$($_ -join ',')]" }
Write-Output -InputObject $null | ForEach-Object { "null record $($null -eq $_)" }
$a = Write-Output -No $null
"$($a.GetType().Name):$($a.Count):$($null -eq $a[0])"
Write-Output -No:$null 42 | ForEach-Object { "null switch $($_.GetType().Name)" }
1,2,3 | Write-Output -No | ForEach-Object { "$($_.GetType().Name) $_" }
@(Write-Output @()).Count
@(Write-Output -No @()).Count
@(@() | Write-Output).Count
$h = @{ InputObject = 7; NoEnumerate = $true }
Write-Output @h
Write-Output @h -InputObject 8
Write-Output -No -- -No | ForEach-Object { $_ -join ',' }
Write-Output -No -Odd:9 | ForEach-Object { $_ -join ',' }
try { Write-Output -InputObject } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Write-Output -No:'bad' 1 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Write-Output -InputObject 1 x } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Write-Output -InputObject 1 -InputObject 2 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Write-Output -Inf Stop } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 1 | Write-Output -InputObject 2 -ea Stop } catch { $_.FullyQualifiedErrorId; $_.CategoryInfo; $_.TargetObject }
$Error.Clear()
1,2 | Write-Output -InputObject 'x' -ea SilentlyContinue
"$? $($Error.Count) $($Error[0].TargetObject)"
$Error.Clear()
1,2 | Write-Output x -ea Ignore
"$? $($Error.Count)"
try { Write-Output -n -No 1 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Write-Output -i 1 (1,2) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Write-Output -Odd:9 -InputObject 1 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
Write-Output -- @{x=5} -No 1 -No:0 | ForEach-Object { "$($_.GetType().Name):$_" }
$a = 'first'
1,2 | Write-Output -ov +a | Out-Null
"$($a.GetType().Name):$($a -join ',')"
$old = $a
Write-Output -No 4 5 -ov +a | Out-Null
"$($old.Equals($a)):$($old.Count):$($old[-1].GetType().Name)"
@() | Write-Output -ov a
"$($a.GetType().Name):$($a.Count)"
$Error.Clear()
1,2 | Write-Output blocked -ea SilentlyContinue -ev trapped
"$($trapped.GetType().Name):$($trapped.Count):$($trapped[0].TargetObject)"
try { 3 | Write-Output blocked -ea Stop -ev trapped } catch { $trapped[0].GetType().Name; $trapped[0].Message }
1 | Write-Output blocked -ea Ignore -ev trapped
$trapped.Count
try { Write-Output 1 -ov 'env:INVALID' } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Write-Output 1 -ov NeverCreated -ev 'env:INVALID' } catch { $_.FullyQualifiedErrorId }
$null -eq $NeverCreated

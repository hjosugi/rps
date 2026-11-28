# rpsh-conformance: level=c2 LANG=C
function Raise-First {
    try { throw 'from child' } catch { "child|$?|$($global:Error.Count)|$_" }
}
Raise-First
$history = $Error
$history.GetType().FullName
(Get-Variable error).Options.ToString()
(Get-Variable error).Value.Equals($history)
$Error.Clear()
try { throw 'outer' } catch {
    "outer|$?|$($_.FullyQualifiedErrorId)|$($history.Count)"
    $_.TargetObject
    $_.CategoryInfo.GetType().FullName
    $_.CategoryInfo.Category
    $_.CategoryInfo.Reason
    $_.CategoryInfo.TargetName
    $_.CategoryInfo.TargetType
    try { throw 'inner' } catch { "inner|$?|$_|$($Error.Count)" }
    "restored|$_"
    try { throw } catch { "again|$?|$_|$($Error.Count)" }
    try { throw } catch { "same|$?|$_|$($Error.Count)" }
}
"after|$_"
foreach ($r in $history) { "entry|$($r.FullyQualifiedErrorId)" }
try { $Error = 5 } catch {
    "$?|$($_.FullyQualifiedErrorId)|$($_.Exception.GetType().FullName)|$($Error.Count)"
}
function Local-Error {
    $Error = 8
    "$Error|$($global:Error.Count)"
    try { throw 'local failure' } catch { "$Error|$($global:Error.Count)" }
}
Local-Error
$history.Clear()
foreach ($n in 1..260) { try { throw "error$n" } catch {} }
"$($history.Count)|$($history[0].FullyQualifiedErrorId)|$($history[-1].FullyQualifiedErrorId)"
$history.Clear()
try { try { throw } catch [System.ArgumentException] { 'unexpected' } finally { 'finally' } }
catch [System.Management.Automation.RuntimeException] { "halt|$?|$($_.FullyQualifiedErrorId)|$($Error.Count)" }
try { Set-Variable -Name Error -Value 'x' -Force -ErrorAction Stop } catch { $_.FullyQualifiedErrorId; $_.CategoryInfo.ToString() }
try { Clear-Variable -Name Error -Force -ErrorAction Stop } catch { $_.FullyQualifiedErrorId; $_.CategoryInfo.ToString() }
try { Remove-Variable -Name Error -Force -ErrorAction Stop } catch { $_.FullyQualifiedErrorId; $_.CategoryInfo.ToString() }
try { New-Variable -Name Error -Value 1 -ErrorAction Stop } catch { $_.FullyQualifiedErrorId; $_.CategoryInfo.ToString() }

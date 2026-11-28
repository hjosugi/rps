# rpsh-conformance: level=c2 LANG=C
function Show-Error($r) {
    $r.FullyQualifiedErrorId
    $r.Exception.GetType().FullName
    $r.Exception.Message
    $r.CategoryInfo
    $r.TargetObject
}
$Error.Clear()
$words=[string[]]@(1,$null,'a b',$true,(1,2))
"$($words.GetType().FullName)|$($words.Count)|$($words -join '|')"
try { exec Get-Command } catch { Show-Error $_ }
try { Switch-Process -WithCommand $null } catch { Show-Error $_ }
try { Switch-Process -WithCommand } catch { Show-Error $_ }
try { Switch-Process -w '/bin/echo' other } catch { Show-Error $_ }
try { Switch-Process /bin/echo -w 2 } catch { Show-Error $_ }
try { Switch-Process -w '/bin/echo' -with 'sh' } catch { Show-Error $_ }
try { exec /bin/echo ('a','b') } catch { Show-Error $_ }
try { Switch-Process $null } catch { Show-Error $_ }
$folder=mktemp -d
try {
    $p="$folder/invalid"
    Set-Content $p 'exit 55'
    chmod 755 $p
    try { Switch-Process $p hello } catch {
        "$($_.FullyQualifiedErrorId)|$($_.CategoryInfo)|$($_.Exception.GetType().FullName)"
        $_.Exception.Message -ceq "Exec call failed with errorno 8 for command line: $p hello"
        $_.TargetObject.GetType().FullName
        $_.TargetObject -is [string[]]
        $_.TargetObject -is [object[]]
        $_.TargetObject[0] -ceq $p
        $_.TargetObject[1]
    }
} finally { Remove-Item $folder -Recurse -Force }
foreach ($act in 'Ignore','SilentlyContinue','Stop') {
    $Error.Clear()
    try {
        'a','b' | Switch-Process -ea $act
        "$act|$?|$($Error.Count)|$($Error[0].TargetObject)"
    } catch { "$act|$?|$($Error.Count)|$($_.FullyQualifiedErrorId)|$($_.TargetObject)" }
}
function exec { "override=$args" }
exec one -w 2 -f:$true -- -g
Remove-Item Function:exec
Test-Path Function:exec

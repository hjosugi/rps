# rpsh-conformance: level=c2
function Read-Header([int]$N, $Tag='x') { "$N/$Tag" }
$before = Get-Command Read-Header
"$($before.Name)|$($before.CommandType)|$($before.Options)|$($before.Visibility)"
"def=<$($before.Definition)>"
& $before 4
function Read-Header { param($Label) "new/$Label" }
"new=<$($before.ScriptBlock)>"
Remove-Item Function:Read-Header
& $before 'after removal'
function Test-Handle {
    function Get-Name { 'outer' }
    $info = Get-Item Function:Get-Name
    & { function Get-Name { 'inner' }; & $info; Get-Name }
}
Test-Handle
filter Add-Tag { "f=$_" }
$f = Get-Command Add-Tag -CommandType Filter
"$($f.Name)/$($f.CommandType)/$($f.GetType().Name)"
'a', 'b' | & $f
(Get-Item Function:Add-Tag).Definition

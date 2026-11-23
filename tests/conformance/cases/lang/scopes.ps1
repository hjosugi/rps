# rpsh-conformance: level=c2
$g = 1
function SetGlobal { $global:g = 2 }
SetGlobal
$g
function Local { $v = 5; $v }
Local
$v -eq $null
$script:s = 3
$s
function ReadParent { $g }
ReadParent

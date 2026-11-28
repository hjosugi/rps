# rpsh-conformance: level=c2 LANG=C
param([string]$Name='default', [int]$Count=1)
if ($Name -eq 'scopes') {
    "enter=$scopeMark|$script:scopeMark|$global:scopeMark"
    $script:scopeMark='file-script'
    function Set-FileScope { $script:scopeMark='nested-script' }
    Set-FileScope
    "finish=$scopeMark|$script:scopeMark|$global:scopeMark"
    return
}
"file=$Name|$Count|$args|$(Split-Path -Leaf $PSCommandPath)|$(Split-Path -Leaf $PSScriptRoot)"
$localMark=$Name
function Get-FileMark { "function:$localMark" }
if ($Name -eq 'exit') { exit 7 }
return 'returned'

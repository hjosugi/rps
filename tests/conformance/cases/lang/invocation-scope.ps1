# rpsh-conformance: level=c2 LANG=C
function Test-Outer($Name) {
    "start=$args|$Name|$($PSBoundParameters.Name)"
    $block={param($Name) $value='child'; function Local-Fn { 'here' }; Set-Alias Local-Alias Write-Output; "$Name|$args|$value"}
    & $block private child
    "after-call=$Name|$args|$value|$(Test-Path Function:Local-Fn)|$(Test-Path Alias:Local-Alias)"
    . $block -Name same extra
    "after-dot=$Name|$args|$value|$(Local-Fn)|$(Local-Alias yes)|$($PSBoundParameters.Name)"
    function Set-Mark($Name) { $value='function'; "inner=$Name|$args" }
    . (Get-Command Set-Mark) fn extra
    "after-function=$Name|$value"
    . 'Write-Output' -InputObject 'cmdlet'
    try { . $null } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
    $warn = . (Get-Command Write-Warning) 'dotted warning' 3>&1
    $warn.Message
}
Test-Outer original arg
"outside=$(Test-Path Function:Local-Fn)|$(Test-Path Alias:Local-Alias)"
filter Count-Items { $seen+=1; return "$seen/$_" }
1,2,3 | & (Get-Command Count-Items)

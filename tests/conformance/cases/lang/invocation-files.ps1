# rpsh-conformance: level=c2 LANG=C
$file='./fixtures/invocation-file with spaces.ps1'
$localMark='parent'
$entry=Get-Command $file -CommandType ExternalScript
"info=$($entry.GetType().Name)|$($entry.CommandType)|$($entry.Name)|$(Split-Path -Leaf $entry.Source)"
& $file -Count 2 -Name called extra
"after-call=$localMark|$(Test-Path Function:Get-FileMark)"
$params=@{ Name='dotted'; Count=4 }
. $entry @params
"after-dot=$localMark|$Name|$Count|$(Get-FileMark)|$(Split-Path -Leaf $PSCommandPath)"
$out = & $entry next 3 rest
"captured=$($out -join '/')"
& $file exit
"call-exit=$?/$LASTEXITCODE"
. $file exit
"dot-exit=$?/$LASTEXITCODE"
"restored=$(Split-Path -Leaf $PSCommandPath)"
$global:scopeMark='root'
function Check-Scopes {
    $scopeMark='caller'
    & $file scopes
    "child=$scopeMark|$script:scopeMark|$global:scopeMark"
    . $file scopes
    "dot=$scopeMark|$script:scopeMark|$global:scopeMark"
}
Check-Scopes
"root=$scopeMark"

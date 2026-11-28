# rpsh-conformance: level=c2
$f = Set-Item -PassThru -Path Function:Get-DriveValue { param($n) "v$n" }
"$($f.Name)/$($f.CommandType)/$($f.Definition)"
Get-DriveValue 7
$original = ${function:Get-DriveValue}
function Set-FromChild {
    Set-Item Function:GET-DRIVEVALUE 'param($n) "changed $n"'
    $a = New-Item Function: -Name Get-ChildOnly -Value { 'child' }
    "$($a.Name)/$($a.Options)"
    Get-ChildOnly
}
Set-FromChild
"changed=$($f.ScriptBlock)"
& $original 8
Get-DriveValue 9
Test-Path Function:Get-ChildOnly
${function:Get-Second} = { 'second' }
$second = Get-Content -LiteralPath Function:Get-Second
"$($second.GetType().Name)/$second"
& $second
filter Select-DriveInput { "record $_" }
$copy = New-Item Function:Select-DriveCopy -Value (Get-Item Function:Select-DriveInput)
"$($copy.Name)/$($copy.CommandType)"
1,2 | Select-DriveCopy
'11','12' | Set-Item -Path Function:Get-Second
Get-Second
$writes = @('21','22' | Set-Content Function:Get-Second -PassThru)
foreach ($write in $writes) { "$($write.GetType().Name)/$($write[0])" }
Get-Second
$reads = @(Get-Content Function:Get-Second -ReadCount 2)
"$($reads.Count)/$($reads[0].GetType().Name)/$($reads[0][0].GetType().Name)"
@(Get-Content Function:Get-Second -TotalCount 0).Count
$x = New-Item Function:Get-Second -Force -Value 'param($n) $n + 1'
Get-Second 4
Clear-Item Function:Get-Second
Test-Path Function:Get-Second
& $x 10
${function:Get-DriveValue} = $null
Test-Path Function:Get-DriveValue
& $f 12
function Get-Preview { 31 }
$before = @(
    Set-Item Function:Get-Preview {32} -WhatIf -PassThru
    New-Item Function:Get-Skipped -Value {0} -WhatIf
    Clear-Item Function:Get-Preview -WhatIf
    Remove-Item Function:Get-Preview -WhatIf
    Add-Content Function:Get-Preview '33' -WhatIf
    Set-Content Function:Get-Preview '34' -WhatIf
)
"preview=$($before.Count)/$(Test-Path Function:Get-Skipped)"
Get-Preview
$WhatIfPreference = $true
Set-Item Function:Get-Preview {35}
Set-Item Function:Get-Preview {36} -WhatIf:$false
$WhatIfPreference = $false
Get-Preview
${function:Get-Single} = @('51')
Get-Single
Remove-Item Function:local:Get-Single
Test-Path Function:Get-Single
Remove-Item Function:Get-Single
Test-Path Function:Get-Single
function Get-Existing { 52 }
& {
    $replica = Set-Item Function:Get-Existing -Value (Get-Command Select-DriveCopy) -PassThru
    $replica.CommandType
    1 | Get-Existing
}
Get-Existing

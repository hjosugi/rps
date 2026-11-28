# rpsh-conformance: level=c2
function Get-Problem($block) {
    try { & $block } catch {
        $_.FullyQualifiedErrorId
        $_.CategoryInfo.ToString()
        $_.Exception.GetType().FullName
        $_.Exception.Message
    }
}
function Test-Drive { 1 }
Get-Problem { New-Item Function:Test-Drive -Value { 2 } -ea Stop }
Get-Problem { New-Item Function:Test-Drive -ea Stop }
Get-Problem { Set-Item Function:Test-Drive -Value 7 -ea Stop }
Get-Problem { ${function:Test-BadValue} = 2 }
Get-Problem { Set-Content Function:Test-Drive -Value '2','3' -ea Stop }
Get-Problem { Add-Content Function:Test-Drive '2' -ea Stop }
Get-Problem { Clear-Content Function:Test-Drive -ea Stop }
Get-Problem { Get-Content Function:Test-Drive -Raw -ea Stop }
Get-Problem { Get-Content Function:Test-Drive -Tail 1 -ea Stop }
Get-Problem { Clear-Item Function:Test-Drive -PassThru -ea Stop }
$Error.Clear()
Get-Content Function:Missing-Drive -ea Ignore
"ignored=$?/$($Error.Count)"
Clear-Item Function:Missing-Drive -ea SilentlyContinue
"quiet=$?/$($Error.Count)/$($Error[0].FullyQualifiedErrorId)"
Set-Content Function:Test-Drive -Value $null -ea SilentlyContinue
"invalid=$?/$($Error[0].FullyQualifiedErrorId)"
Test-Drive
Set-Content Function:Test-Drive -Value @($null)
Test-Path Function:Test-Drive

# rpsh-conformance: level=c2
if ($args[0] -eq 'file') {
    function script:Get-Level { 'file' }
    function Set-NestedLevel {
        function script:Get-Level { 'updated file' }
        function global:Get-FromFile { 'global from file' }
        script:Get-Level
        global:Get-Level
    }
    Set-NestedLevel
    Get-Level
    return
}
function Get-Level { 'root' }
function Get-Shadow { 'root shadow' }
function Read-OuterScope {
    function local:Get-Level { 'local' }
    function private:Get-Shadow { 'private shadow' }
    function private:Get-Hidden { 'hidden' }
    function Read-InnerScope {
        Get-Level
        Get-Shadow
        try { Get-Hidden } catch { $_.FullyQualifiedErrorId }
        (Get-Command Get-Hidden).Options
        Test-Path Function:Get-Hidden
        function script:Get-Level { 'updated root' }
        global:Get-Level
    }
    Get-Level; Get-Shadow; Get-Hidden; Read-InnerScope
    Get-Level; Get-Shadow
}
Read-OuterScope
Get-Level; Get-Shadow
& $PSCommandPath 'file'
Get-Level; Get-FromFile
function Remove-InChild { Remove-Item Function:Get-FromFile }
Remove-InChild
Test-Path Function:Get-FromFile

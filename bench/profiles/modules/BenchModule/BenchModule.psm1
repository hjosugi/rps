# A small module for the import profile; it defines and exports one function.
function Get-BenchModuleInfo {
    [pscustomobject]@{ Name = 'BenchModule'; Version = '1.0' }
}
Export-ModuleMember -Function Get-BenchModuleInfo

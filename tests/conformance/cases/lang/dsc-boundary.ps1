# rpsh-conformance: level=c2
foreach ($name in 'Invoke-DscResource','Get-DscResource','Start-DscConfiguration') {
    (Get-Command -Name $name -ErrorAction SilentlyContinue | Measure-Object).Count
}
Write-Output configuration
function configuration { 'command after pipe' }
1 | configuration

# rpsh-conformance: level=c2
Invoke-Expression '1 + 2'
$cmd = '"hi"'
iex $cmd
Set-Variable -Name x -Value 42
$x
(Get-Variable -Name x).Value
Remove-Variable -Name x
$x -eq $null

# rpsh-conformance: level=c2
$a = 2
$b = 3
$a * $b
$name = 'world'
"hello $name"
$a += 10
$a
$undefined -eq $null
(Set-Variable temp 7 -PassThru).Name
(Set-Variable temp 8 -PassThru).Value
$temp
New-Variable nv 5
$nv
(New-Variable nv2 6 -PassThru).Value
Clear-Variable nv
$null -eq $nv
$nv
(Get-Variable nv2 -ValueOnly) + 1
(Get-Variable 'nv*' | ForEach-Object Name) -join ','
(Get-Variable 'nv2*' -ValueOnly) -join ','

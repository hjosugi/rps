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
New-Variable rv1 1
New-Variable rv2 2
Remove-Variable rv*
(Get-Variable 'rv*' | Measure-Object).Count
New-Variable cv1 1
New-Variable cv2 2
Clear-Variable cv*
($null -eq $cv1) -and ($null -eq $cv2)
New-Variable mv1 1
New-Variable mv2 2
(Get-Variable mv1, mv2 | ForEach-Object Name | Sort-Object) -join ','
(Get-Variable -Name mv1, mv2 -ValueOnly) -join ','
New-Variable rm1 1
New-Variable rm2 2
Remove-Variable rm1, rm2
(Get-Variable 'rm*' | Measure-Object).Count
New-Variable c1 1
New-Variable c2 2
Clear-Variable c1, c2
($null -eq $c1) -and ($null -eq $c2)
Remove-Variable c1, c2

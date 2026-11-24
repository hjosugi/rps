# rpsh-conformance: level=c2
$o = New-Object PSObject -Property ([ordered]@{A=1;B=2})
$o.A
$o.B
$o.psobject.Properties.Name -join ','
(Get-Command New-Object).CommandType

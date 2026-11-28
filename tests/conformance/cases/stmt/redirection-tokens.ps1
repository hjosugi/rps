# rpsh-conformance: level=c2
Write-Output abc>def
Write-Output 1 abc>def
Write-Output 7>name
Write-Output 12>name
Write-Output '2>&1'
Write-Output '*>'
Write-Warning 'merged' 3>&1
$record = Write-Warning 'captured' 3>&1
$record.GetType().FullName
$record.Message

# rpsh-conformance: level=c2
if (Test-Path rpsh-clear.txt) { Remove-Item rpsh-clear.txt }
Set-Content rpsh-clear.txt -Value 'abc'
Clear-Content rpsh-clear.txt
(Get-Item rpsh-clear.txt).Length
Remove-Item rpsh-clear.txt

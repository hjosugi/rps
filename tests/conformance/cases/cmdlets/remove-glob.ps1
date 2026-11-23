# rpsh-conformance: level=c2
Set-Content rpsh-g1.tmp -Value 'a'
Set-Content rpsh-g2.tmp -Value 'b'
Remove-Item rpsh-*.tmp
Test-Path rpsh-g1.tmp
Test-Path rpsh-g2.tmp

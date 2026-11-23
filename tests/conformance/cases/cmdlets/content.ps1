# rpsh-conformance: level=c2
Get-Content fixtures/data.txt -TotalCount 2
Test-Path fixtures/data.txt
Test-Path fixtures/nope.txt
Join-Path a b
Split-Path fixtures/data.txt -Leaf
Split-Path fixtures/data.txt -Parent

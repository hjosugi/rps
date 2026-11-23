# rpsh-conformance: level=c2
Test-Path fixtures/data.txt -PathType Leaf
Test-Path fixtures -PathType Container
Test-Path fixtures/data.txt -PathType Container
(Resolve-Path fixtures/data.txt).Path -eq (Join-Path (Get-Location).Path 'fixtures/data.txt')
Test-Path fixtures/*.txt
Test-Path fixtures/*.nope
Split-Path fixtures/data.txt -Extension
Split-Path fixtures/data.txt -LeafBase
Test-Path fixtures/tree/*.txt -PathType Leaf
Test-Path fixtures/tree/*.txt -PathType Container

# rpsh-conformance: level=c2
(Get-FileHash fixtures/data.txt).Hash
(Get-FileHash fixtures/data.txt).Algorithm
(Get-FileHash fixtures/data.txt, fixtures/tree/a.txt | ForEach-Object Hash) -join ','
(Get-FileHash fixtures/tree/*.txt).Count

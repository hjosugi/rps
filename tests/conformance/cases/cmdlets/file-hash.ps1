# rpsh-conformance: level=c2
(Get-FileHash fixtures/data.txt).Hash
(Get-FileHash fixtures/data.txt).Algorithm
(Get-FileHash fixtures/data.txt, fixtures/tree/a.txt | ForEach-Object Hash) -join ','
(Get-FileHash fixtures/tree/*.txt).Count
(Get-FileHash fixtures/data.txt -Algorithm SHA1).Hash
(Get-FileHash fixtures/data.txt -Algorithm MD5).Hash
(Get-FileHash fixtures/data.txt -Algorithm SHA1).Algorithm
(Get-FileHash fixtures/data.txt -Algorithm MD5).Algorithm

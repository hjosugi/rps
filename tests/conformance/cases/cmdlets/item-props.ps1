# rpsh-conformance: level=c2
(1..5 | Select-Object -SkipLast 2) -join ','
(Get-Item fixtures/data.txt).Name
(Get-Item fixtures/data.txt).Length
(Get-Item fixtures/data.txt).Extension
('ab', 'cde' | ForEach-Object -MemberName Length) -join ','
(Get-Item fixtures/data.txt).BaseName

# rpsh-conformance: level=c2
(1..5 | Select-Object -SkipLast 2) -join ','
(Get-Item fixtures/data.txt).Name
(Get-Item fixtures/data.txt).Length
(Get-Item fixtures/data.txt).Extension
('ab', 'cde' | ForEach-Object -MemberName Length) -join ','
(Get-Item fixtures/data.txt).BaseName
(Get-Item fixtures/tree/*.txt | ForEach-Object Name | Sort-Object) -join ','
(Get-Item fixtures/tree).PSIsContainer
(Get-Item fixtures/data.txt).PSIsContainer
(Get-Item fixtures/data.txt, fixtures/tree/a.txt | ForEach-Object Name) -join ','
(Get-Item fixtures/data.txt).Name
$p = Get-ItemProperty -Path fixtures/data.txt -Name Name, Length
$p.Name
$p.Length
(Get-ItemProperty fixtures/data.txt).Name
(Get-ItemProperty fixtures/data.txt).Length
(Get-Command Get-ItemProperty).CommandType
$env:RPSH_ITEM = 'v'
(Get-Item Env:RPSH_ITEM).Name
(Get-Item Env:RPSH_ITEM).Value
$iv = 1
(Get-Item Variable:iv).Name
(Get-Item Variable:iv).Value
(Get-Item Alias:gc).Name
(Get-Item Alias:gc).Definition
function LocalItem {}
(Get-Item Function:LocalItem).Name
(Get-Item fixtures/data.txt).Mode
(Get-Item fixtures/tree).Mode

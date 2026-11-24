# rpsh-conformance: level=c2
(Get-ChildItem fixtures/tree -Recurse -File | ForEach-Object Name | Sort-Object) -join ','
(Get-ChildItem fixtures/tree -Filter '*.txt' -Name | Sort-Object) -join ','
(Get-ChildItem fixtures/tree -Recurse -File | ForEach-Object Length | Measure-Object -Sum).Sum
(Get-ChildItem fixtures/tree -Recurse -Name | Sort-Object) -join ','
(Get-ChildItem fixtures/tree -Directory -Name | Sort-Object) -join ','
(Get-ChildItem fixtures/tree -File -Name | Sort-Object) -join ','
(Get-ChildItem fixtures/tree -Name) -join ','
(Get-ChildItem fixtures/tree -Recurse -File -Name -Exclude 'b.txt' | Sort-Object) -join ','
(Get-ChildItem fixtures/tree -Recurse -File -Name -Include 'b.txt' | Sort-Object) -join ','
(Get-ChildItem fixtures/tree -Recurse -Name -Depth 0 | Sort-Object) -join ','
(Get-ChildItem fixtures/tree -Recurse -Name -Depth 1 | Sort-Object) -join ','
(Get-ChildItem fixtures/tree -Name -Depth 1 | Sort-Object) -join ','
(Get-ChildItem fixtures/tree/*.txt -Name) -join ','
(Get-ChildItem fixtures/tree/* -Name | Sort-Object) -join ','
(Get-ChildItem fixtures/tree/*.txt).Length
(Get-ChildItem fixtures/tree | ForEach-Object PSIsContainer) -join ','
(Get-ChildItem fixtures/tree -File | ForEach-Object { "$($_.BaseName)$($_.Extension)" }) -join ','

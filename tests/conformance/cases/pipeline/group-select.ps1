# rpsh-conformance: level=c2
'a', 'b', 'a', 'c', 'b' | Group-Object | ForEach-Object { "$($_.Name)=$($_.Count)" }
'a', 'A', 'b' | Select-Object -Unique
'c', 'a', 'b' | Sort-Object -Descending
'a', 'B', 'A', 'b' | Sort-Object

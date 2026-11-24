# rpsh-conformance: level=c2
1 -eq 1
'a' -eq 'A'
'a' -ceq 'A'
1 -lt 2
'10' -gt 9
1, 2, 3 -gt 1
1, 2, 3 -contains 2
2 -in 1, 2, 3
'abc' -like 'a*'
'abc' -match '^(a)(b)'
$Matches[2]
'abc' -replace 'b', 'X'
'a,b,c' -split ','
'a', 'b' -join '+'
$null -eq $null
'gc' -contains 'gc'
'gc' -contains 'x'
$null -contains 'a'
'abc' -like 'A*'
'abc' -clike 'A*'
'abc' -notlike 'x*'
'abc' -cnotlike 'A*'
'abc' -match 'A'
'abc' -cmatch 'A'
'abc' -notmatch 'z'
1 -is [int]
'x' -is [string]
'5' -as [int]
'x' -as [int]

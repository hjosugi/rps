# rpsh-conformance: level=c2
1, 2, 3 -ne 2
1, 2, 3 -notcontains 2
4 -notin 1, 2, 3
'abc' -eq 'ABC'
'ABC' -ceq 'abc'
1 -is [int]
1 -isnot [string]
'5' -as [int]
'x' -as [int]
1 -as [string]
2147483648 -gt 2147483647

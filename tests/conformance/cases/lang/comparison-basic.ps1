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
$null -eq $null
1 -and 0
0 -or 2
-not $false
1 -band 3
1 -shl 4

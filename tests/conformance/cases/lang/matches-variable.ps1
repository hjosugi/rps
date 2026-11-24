# rpsh-conformance: level=c2
'x' -match 'z'
$null -eq $Matches
'a1b22' -match '(\d+)'
$Matches[0]
$Matches[1].Length
'a1b22' -match 'zz'
$null -eq $Matches

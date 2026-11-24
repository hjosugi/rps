# rpsh-conformance: level=c2
(1 -eq 1) ? 'yes' : 'no'
(1 -eq 2) ? 'yes' : 'no'
$null ?? 'fallback'
'value' ?? 'fallback'
$a = $null
$a ??= 'set'
$a
$b = 'keep'
$b ??= 'change'
$b

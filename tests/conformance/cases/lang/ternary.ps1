# rpsh-conformance: level=c2
# Ternary and null-coalescing operators.
(1 -eq 1) ? 'yes' : 'no'
(1 -eq 2) ? 'yes' : 'no'
$null ?? 'fallback'
'x' ?? 'fallback'
$x = $null
$x ??= 'set'
$x
$y = 'already'
$y ??= 'other'
$y

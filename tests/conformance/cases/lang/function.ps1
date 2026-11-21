# rpsh-conformance: level=c2
function Add($a, $b = 10) { $a + $b }
Add 1 2
Add 5
function Get-Square { param([int]$n) return $n * $n }
Get-Square -n 4
function Sum-Input { begin { $t = 0 } process { $t += $_ } end { $t } }
1..4 | Sum-Input
filter Double { $_ * 2 }
1, 2 | Double
function Outer { $v = 'outer'; Inner }
function Inner { $v }
Outer

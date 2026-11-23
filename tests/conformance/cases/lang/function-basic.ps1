# rpsh-conformance: level=c2
function Add($a, $b = 10) { $a + $b }
Add 1 2
Add 5
function Get-Square { param([int]$n) return $n * $n }
Get-Square -n 4
function Greet { "hi $args" }
Greet a b
function Outer { $v = 'outer'; Inner }
function Inner { $v }
Outer

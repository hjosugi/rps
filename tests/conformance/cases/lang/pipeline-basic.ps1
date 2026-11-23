# rpsh-conformance: level=c2
filter Double { $_ * 2 }
1, 2, 3 | Double
function Sum { $t = 0; foreach ($x in $input) { $t += $x }; $t }
1..4 | Sum
'abc' | Double | Double

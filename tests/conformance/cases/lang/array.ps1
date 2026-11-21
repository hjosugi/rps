# rpsh-conformance: level=c2
$a = 1, 2, 3
$a.Count
$a[0]
$a[-1]
$a[1..2] -join ','
$a += 4
$a -join ','
@().Count
@(5).Count
(, 1).Count
$b = 1..5
$b[1, 3] -join ','

# rpsh-conformance: level=c2
$x = 'a', 'b', 'c'
$y = 'b', 'c', 'd'
Compare-Object $x $y | ForEach-Object { "$($_.SideIndicator)$($_.InputObject)" }

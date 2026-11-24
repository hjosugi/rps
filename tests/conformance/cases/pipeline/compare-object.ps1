# rpsh-conformance: level=c2
$x = 'a', 'b', 'c'
$y = 'b', 'c', 'd'
Compare-Object $x $y | ForEach-Object { "$($_.SideIndicator)$($_.InputObject)" }
Compare-Object $x $y -IncludeEqual | ForEach-Object { "$($_.SideIndicator)$($_.InputObject)" }
Compare-Object 'a','B' 'A','b' -CaseSensitive | ForEach-Object { "$($_.SideIndicator)$($_.InputObject)" }
'---'
Compare-Object 'a','B' 'A','b' | ForEach-Object { "$($_.SideIndicator)$($_.InputObject)" }
$x = [pscustomobject]@{Name='a'; V=1}
$y = [pscustomobject]@{Name='b'; V=2}
Compare-Object $x $y -Property Name | ForEach-Object { "$($_.Name)$($_.SideIndicator)" }
Compare-Object $x $y -Property Name -IncludeEqual | ForEach-Object { "$($_.Name)$($_.SideIndicator)" }

$x = 5
$o = [pscustomobject]@{ Name = 'n' }
"x=$x"
"sum=$($x + 1)"
"prop=$($o.Name) not $o.Name"
"tab`there"
"dollar `$x"
"${x}th"
@"
here $x
"quoted"
"@
@'
literal $x
'@

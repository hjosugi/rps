# rpsh-conformance: level=c2 LANG=C
function Show-Bound($a,$b) { "$a|$b|" + ($args -join '|') }
function Forward-Args { Show-Bound @args }
function Print-Args { $args.Count; foreach ($x in $args) { "$($null -eq $x)[$x]" } }
function Forward-Twice { Forward-Args @args }
function Forward-Copy { $copy=$args; ./fixtures/argv-dump @copy }
$v=$null; Print-Args before @v after
$d=@{a=1}; Show-Bound @d -a 42; Show-Bound -a 42 @d; Show-Bound @d 2
$d=[ordered]@{a='x y';b=''}; Forward-Twice @d
$x=@('-a','VAL'); Show-Bound @x tail
Forward-Args -a:4 -b 9; Forward-Args -b 9 -a:4; Forward-Args -extra q -b z tail
$x=@('x y',2,('foo','bar')); Forward-Copy -one @x -two:3
$v='a🌲'; Print-Args @v
$d=@{Name='pwd'}; (Get-Command @d).Name
$d=[ordered]@{Top=2;Descending=$true}; 1,2,3 | Sort-Object @d; 1,2,3 | Sort-Object @d -Descending:$false; 1,2,3 | Sort-Object -Descending:$false @d
$d=@{a=1}
try { Show-Bound @d @d } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Show-Bound -a 1 -a 2 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }

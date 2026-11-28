# rpsh-conformance: level=c2
$values = @(1; 2, 3; Write-Output 4)
"values=$values/$($values.GetType().Name)"
@().Count
@($null).Count
@(& {}).Count
$rows = @(,('a', 'b'); 'c')
"rows=$($rows.Count)/$($rows[0].Count)"
$nested = @(foreach ($value in 2, 4) { $value; $value + 1 })
"nested=$nested"

# rpsh-conformance: level=c2
[ordered]@{ a = 1; b = @(1, 2); c = 'x' } | ConvertTo-Json -Compress
$o = '{"x":1,"y":[1,2,{"z":null}]}' | ConvertFrom-Json
$o.x
$o.y.Count
$null -eq $o.y[2].z
('a', 'b' | ConvertTo-Json -AsArray -Compress)
(1 | ConvertTo-Json -AsArray -Compress)

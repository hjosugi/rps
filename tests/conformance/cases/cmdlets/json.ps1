# rpsh-conformance: level=c2
[ordered]@{ a = 1; b = @(1, 2); c = 'x' } | ConvertTo-Json -Compress
$o = '{"x":1,"y":[1,2,{"z":null}]}' | ConvertFrom-Json
$o.x
$o.y.Count
$null -eq $o.y[2].z
('a', 'b' | ConvertTo-Json -AsArray -Compress)
(1 | ConvertTo-Json -AsArray -Compress)
$h = '{"a":1,"b":{"c":2}}' | ConvertFrom-Json -AsHashtable
$h.GetType().Name
$h.a
$h.b.GetType().Name
$h.b.c
$h.ContainsKey('a')
$p = ConvertFrom-Json '{"x":3}'
$p.x

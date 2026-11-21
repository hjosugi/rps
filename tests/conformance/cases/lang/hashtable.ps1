# rpsh-conformance: level=c2
$h = [ordered]@{ one = 1; two = 2 }
$h.one
$h['two']
$h.three = 3
$h.Count
$h.Keys -join ','
$t = @{ Name = 'x' }
$t.ContainsKey('name')
$t.Missing -eq $null

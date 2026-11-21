# Repeated array += (copying) at a larger size.
$items = @()
for ($i = 0; $i -lt 10000; $i++) { $items += $i }
$items.Count
$items[-1]

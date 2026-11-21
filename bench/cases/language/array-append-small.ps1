# Repeated array += (copying) at a small size.
$items = @()
for ($i = 0; $i -lt 2000; $i++) { $items += $i }
$items.Count
$items[-1]

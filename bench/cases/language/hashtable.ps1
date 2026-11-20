# Hashtable insertion and lookup with string keys.
$table = @{}
for ($i = 0; $i -lt 100000; $i++) { $table["key$i"] = $i }
$total = 0
for ($i = 0; $i -lt 100000; $i += 7) { $total += $table["key$i"] }
$table.Count
$total

# Hashtable insertion and lookup at 1/10 the size of hashtable.
$table = @{}
for ($i = 0; $i -lt 10000; $i++) { $table["key$i"] = $i }
$total = 0
for ($i = 0; $i -lt 10000; $i += 7) { $total += $table["key$i"] }
$table.Count
$total

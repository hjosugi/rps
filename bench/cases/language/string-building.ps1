# String interpolation, joining, splitting and method calls.
$parts = foreach ($i in 1..50000) { "item-$i" }
$text = $parts -join ','
$text.Length
($text -split ',').Count
$text.Substring(0, 20).ToUpper()

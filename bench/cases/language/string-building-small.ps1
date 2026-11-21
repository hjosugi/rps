# String interpolation, joining, splitting and method calls at 1/10 the size.
$parts = foreach ($i in 1..5000) { "item-$i" }
$text = $parts -join ','
$text.Length
($text -split ',').Count
$text.Substring(0, 20).ToUpper()

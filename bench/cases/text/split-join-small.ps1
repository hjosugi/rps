# -join and -split over a smaller set of strings.
$parts = foreach ($i in 1..10000) { "part$i" }
$text = $parts -join ','
$text.Length
($text -split ',').Count
($text -split ',' -join ';').Length

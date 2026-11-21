# -join and -split over 100k strings.
$parts = foreach ($i in 1..100000) { "part$i" }
$text = $parts -join ','
$text.Length
($text -split ',').Count
($text -split ',' -join ';').Length

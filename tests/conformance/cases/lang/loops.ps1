for ($i = 0; $i -lt 3; $i++) { $i }
foreach ($x in 'a', 'b', 'c') { if ($x -eq 'b') { continue }; $x }
$n = 0
while ($true) { $n++; if ($n -ge 3) { break } }
$n
do { $n-- } until ($n -le 0)
$n
:outer foreach ($a in 1..3) { foreach ($b in 1..3) { if ($b -eq 2) { continue outer }; "$a$b" } }

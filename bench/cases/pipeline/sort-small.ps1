# Sort-Object over a smaller generated set.
$seed = 12345
$items = foreach ($i in 1..2000) {
    $seed = ($seed * 1103515245 + 12345) % 2147483648
    [pscustomobject]@{ Id = $i; Key = $seed % 100 }
}
$sorted = $items | Sort-Object Key, Id
$sorted[0].Id
$sorted[-1].Id

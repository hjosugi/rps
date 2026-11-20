# Sort-Object and Group-Object over objects with a computed key.
$seed = 12345
$items = foreach ($i in 1..20000) {
    $seed = ($seed * 1103515245 + 12345) % 2147483648
    [pscustomobject]@{ Id = $i; Key = $seed % 100 }
}
$sorted = $items | Sort-Object Key, Id
$sorted[0].Id
$sorted[-1].Id
($items | Group-Object Key).Count

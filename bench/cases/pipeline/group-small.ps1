# Group-Object over a smaller generated set.
$items = foreach ($i in 1..2000) { [pscustomobject]@{ Id = $i; Key = $i % 100 } }
($items | Group-Object Key).Count

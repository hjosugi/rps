# A switch statement per loop iteration.
$total = 0
for ($i = 0; $i -lt 20000; $i++) {
    switch ($i % 5) {
        0 { $total += 1 }
        1 { $total += 2 }
        2 { $total += 3 }
        3 { $total += 4 }
        default { $total += 5 }
    }
}
$total

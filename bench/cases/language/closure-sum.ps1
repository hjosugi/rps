# A closure that keeps state across script block invocations.
$make = { $state = @{ total = 0 }; { param($n) $state.total += $n; $state.total }.GetNewClosure() }
$counter = & $make
$last = 0
for ($i = 1; $i -le 20000; $i++) { $last = & $counter $i }
$last

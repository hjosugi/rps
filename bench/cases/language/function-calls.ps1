# Recursive function calls: parameter binding and scope setup per call.
function Fib($n) { if ($n -lt 2) { return $n }; return (Fib ($n - 1)) + (Fib ($n - 2)) }
Fib 18

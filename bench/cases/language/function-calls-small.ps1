# Recursive calls at a smaller size than function-calls.
function Fib($n) { if ($n -lt 2) { return $n }; return (Fib ($n - 1)) + (Fib ($n - 2)) }
Fib 15

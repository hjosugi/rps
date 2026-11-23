# rpsh-conformance: level=c2
$s = '😀あ'
$s.Length
$high = $s[0]
$high -ceq $s[0]
$high -ceq $s[1]
$s[2]
$high
$s[-1]

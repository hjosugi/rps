# rpsh-conformance: level=c2
$m = 'hello world', 'foo' | Measure-Object -Word -Line -Character
"$($m.Words) $($m.Lines) $($m.Characters)"

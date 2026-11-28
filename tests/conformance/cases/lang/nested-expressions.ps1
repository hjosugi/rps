# rpsh-conformance: level=c2
class ExpressionValue {
    [int] $Value
    ExpressionValue([int]$n) { $this.Value = $n }
    [int] Read() { return $this.Value }
}
$seen = [Collections.ArrayList]::new()
function Observe($value) {
    $null = $seen.Add($value)
    return $value
}
$n = [ExpressionValue]::new(7)
"$($n.Read() + (Observe 2))|$([int]($n.Read() * 3))"
$obj = [pscustomobject]@{ Left = $n.Read(); Right = (Observe 4) }
"$($obj.Left)|$($obj.Right)"
$ordered = [ordered]@{ x = (Observe a); y = $n.Read() }
"$($ordered.x)|$($ordered.y)"
$false -and (Observe skipped)
$true -or (Observe skipped)
$true -and (Observe and)
$false -or (Observe or)
$true ? (Observe yes) : (Observe skipped)
$null ?? (Observe fallback)
(Observe present) ?? (Observe skipped)
-not (Observe 0)
(-join @('a', (Observe b)))
function Result($a = (Observe 5), $b = ($a + $n.Read())) { "$a/$b" }
Result
Result 3
"log $($seen -join ',')"

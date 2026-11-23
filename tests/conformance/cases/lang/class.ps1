# rpsh-conformance: level=c2
class Point {
    [int]$X
    [int]$Y
    Point([int]$x, [int]$y) { $this.X = $x; $this.Y = $y }
    [int] Sum() { return $this.X + $this.Y }
}
$p = [Point]::new(3, 4)
$p.X
$p.Y
$p.Sum()
$p.X = 10
$p.Sum()

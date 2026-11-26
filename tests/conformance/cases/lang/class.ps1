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
class Animal {
    [string] $Name
    [string] Speak() { return "$($this.Name) speaks" }
}
class Dog : Animal {
    [string] Bark() { return "$($this.Name) barks" }
}
$d = [Dog]::new()
$d.Name = 'Rex'
$d.Speak()
$d.Bark()
$d.GetType().Name
$d -is [Dog]
$d -is [Animal]
$d -is [string]
class BaseC { [int] $X; BaseC() { $this.X = 1 } }
class DerivedC : BaseC { [int] $Y; DerivedC() { $this.Y = 2 } }
$dc = [DerivedC]::new()
$dc.X
$dc.Y
class ImplicitC : BaseC { [int] $Z }
$ic = [ImplicitC]::new()
$ic.X
$ic.Z

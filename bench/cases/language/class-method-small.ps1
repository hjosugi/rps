# PowerShell class instantiation and method calls, at a small size.
class Counter {
    [long] $Value
    Counter() { $this.Value = 0 }
    [void] Add([int] $n) { $this.Value += $n }
}
$counter = [Counter]::new()
for ($i = 0; $i -lt 20000; $i++) { $counter.Add($i) }
$counter.Value

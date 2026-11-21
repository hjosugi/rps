# PowerShell class instantiation and method calls, at 10x the small case.
class Counter {
    [long] $Value
    Counter() { $this.Value = 0 }
    [void] Add([int] $n) { $this.Value += $n }
}
$counter = [Counter]::new()
for ($i = 0; $i -lt 200000; $i++) { $counter.Add($i) }
$counter.Value

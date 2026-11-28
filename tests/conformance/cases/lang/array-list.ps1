# rpsh-conformance: level=c2 LANG=C
$a = [System.Collections.ArrayList]::new(1)
$a.GetType().FullName
$a.ToString()
$a.Count
$a.Capacity
[bool]$a
$a.Add(0)
[bool]$a
$a.Add('two')
$a.Capacity
$alias = $a
$alias.Insert(1,'middle')
$a.Count
$a.Capacity
$a -join '|'
$a[-1]
$null -eq $a[99]
$a[1] = 'changed'
$alias.IndexOf('changed')
$alias.Contains('Changed')
$alias.Remove(0)
$alias.RemoveAt(0)
$a -join '|'
$b = [Collections.ArrayList]@('one','two')
$b.AddRange(@('three','four'))
$b.Capacity
$c = $b.Clone()
$c.Remove('two')
$b.Count
$c.Count
$b.ToArray().GetType().FullName
$b.ToArray() -join ','
$b[-1,0] -join ','
$b -eq 'TWO'
$b -contains 'TWO'
$b.Clear()
$c -join ','
$b.Count
$b.Capacity
foreach ($item in $a) { "foreach=$item" }
$mapped = $c | ForEach-Object { "item=$_" }
$mapped -join ','
$c.IsReadOnly
$c.IsFixedSize
$c.IsSynchronized
$c.Equals($c.SyncRoot)
$c -is [Collections.IList]
$c -is [array]
$c -is [object]
$c.Capacity = 9
$c.InsertRange(0, @('left', 'right'))
$c.Reverse()
$c.TrimToSize()
$c.Capacity
$c -join ','
$c.Capacity = $c.Count
$c.set_Item(0, 'first')
$c.get_Item(0)
try { $c.Capacity = 1 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { [Collections.ArrayList]::new('bad') } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { $a.RemoveAt(-1) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { $a.Insert(5,0) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { [Collections.ArrayList]::new(-1) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }

# rpsh-conformance: level=c2
$l = [System.Collections.Generic.List[object]]::new()
$l.GetType().FullName
$l.GetType().Name
$l.ToString()
$l -is [Collections.Generic.List[object]]
$l -is [System.Collections.IList]
$l -is [System.Array]
$l -is [Collections.ArrayList]
@($l.Add(7)).Count
$l.Add('abc')
"$($l.Count)/$($l.Capacity)/$($l -join ',')"
$alias = $l.SyncRoot
$alias[-1] = 'hello'
$l.Insert(1, 'second')
$l.AddRange([object[]](12, 13))
$l.Reverse()
"$($l -join ',')/$($l.IndexOf('second'))/$($l.Contains(13))"
$l.Remove('second')
$l.Remove(99)
$l.Length -join ','
$l.set_Item(1, 55)
$l.get_Item(1)
$l.ToArray().GetType().FullName
$copy = [Collections.Generic.List[System.Object]]::new($l.ToArray())
$copy.RemoveAt(0)
$copy.TrimExcess()
"$($l.Count),$($copy.Count),$($copy.Capacity)"
$cast = [Collections.Generic.List[object]]'x'
"$($cast.Count),$($cast.Capacity),$($cast[0])"
$negative = $copy[-1]
$negative
$items = @(foreach ($i in $l) { $i })
$items -join ','
$l.Clear()
$l.Count
$l.Capacity = 0
$l.Capacity
$l.GetType().Namespace
$l.GetType().IsGenericType
([Collections.ArrayList]).IsGenericType
$a = [Collections.ArrayList]@(1,2)
$single = [Collections.Generic.List[object]]$a
"$($single.Count):$($single[0].GetType().Name)"
$l.AddRange([string[]]('a','bb'))
$l.AddRange($l)
$l -join ','
([Collections.ArrayList]$l).Count
try { $l.AddRange($a) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { $l.AddRange($true) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { $l.InsertRange(0,$null) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { $null = [Collections.Generic.List[object]]::new(-1) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { $null = [Collections.Generic.List[object]]::new($a) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }

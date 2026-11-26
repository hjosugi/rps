# rpsh-conformance: level=c2
$o = [pscustomobject]@{ 'b' = 2; 'a' = 1 }
$p = $o.psobject
-join $p.TypeNames
-join $o.pstypenames
$props = $p.Properties
-join $props.Name
$o -eq $o
$o -eq ([pscustomobject]@{ 'b' = 2; 'a' = 1 })
@($null).Count
([pscustomobject]@{A=1;B=2}).Count
([ordered]@{A=1;B=2}).Count
([pscustomobject]@{A=1}).Length
(1).Length
([ordered]@{A=1;B=2}).Length
$null.Count
$null.Length
$true.Length

# rpsh-conformance: level=c2
$x=7
$ref=Get-Variable x
$x=8
"$($ref.GetType().Name):$($ref.Name)=$($ref.Value)"
$ref.Value=9
"$x/$([object]::ReferenceEquals($ref,(Get-Variable x)))"
$var=[psvariable]::new('sample',(1,2,3,4,5))
"$($var.GetType().FullName)|$($var.Options)|$($var.Visibility)|$($var.Attributes.GetType().Name)|$($var.Module -eq $null)"
$var.PSObject.Properties.Name -join ','
$var | Out-String -Width 52
$var | Format-List | Out-String
$l=[Collections.Generic.List[psvariable]]::new()
$l.Add($var)
$l.Add('fromString')
$l.Add($null)
"$($l.Count)|$($l.Capacity)|$($l[1].Name)|$($l[2] -eq $null)"
$copy=[Collections.Generic.List[psvariable]]::new($l)
"$($copy.Count)|$($copy.Capacity)|$([object]::ReferenceEquals($copy[0],$l[0]))"
$ro=[psvariable]::new('locked',3,'ReadOnly')
try { $ro.Value=4 } catch { "$($_.FullyQualifiedErrorId)|$($_.Exception.Message)" }
foreach($action in { [psvariable]::new('') },{ [psvariable]@{Name='x'} },{ $l.Add('') },{ $l.Add(@(1,2)) }) {
    try { & $action } catch {
        $inner=$_.Exception.InnerException
        "$($_.FullyQualifiedErrorId)|$($inner -eq $null ? '' : $inner.GetType().Name)|$($_.Exception.Message)"
    }
}

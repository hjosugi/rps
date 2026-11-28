# rpsh-conformance: level=c2
foreach ($n in 'Write-Output','Sort-Object','Switch-Process') {
    $c = Get-Command -Name $n
    $m = [System.Management.Automation.CommandMetadata]::new($c)
    $all = [System.Management.Automation.CommandMetadata]::new($c,$true)
    "$($m.Name)|$($m.CommandType.FullName)|$($m.DefaultParameterSetName)|$($m.RemotingCapability)|$($m.ConfirmImpact)|$($m.HelpUri)"
    "$($m.SupportsShouldProcess)|$($m.PositionalBinding)|$($m.SupportsPaging)|$($m.SupportsTransactions)"
    "$($m.Parameters.Count)|$($all.Parameters.Count)|$($c.Parameters.Count)"
    $m.Parameters.Keys -join ','
    foreach ($p in $m.Parameters.Values) {
        "$($p.Name) $($p.ParameterType.FullName) $($p.SwitchParameter) $($p.IsDynamic)"
        foreach ($key in $p.ParameterSets.Keys) {
            $s = $p.ParameterSets[$key]
            "$key $($s.Position) $($s.IsMandatory) $($s.ValueFromPipeline) $($s.ValueFromRemainingArguments)"
        }
        ($p.Attributes | ForEach-Object { $_.GetType().Name }) -join ','
    }
    [object]::ReferenceEquals($c.Parameters,$c.Parameters)
    [object]::ReferenceEquals($m.Parameters,$c.Parameters)
}
$c = Get-Command Write-Output
$p = $c.Parameters['OUTVARIABLE']
"$($p.GetType().FullName) $($p.Aliases.GetType().FullName) $($p.Aliases.Count) $($p.Aliases[0])"
$p.Aliases.Add('captured')
$c.Parameters.OutVariable.Aliases -join ','
$m = [System.Management.Automation.CommandMetadata]::new((Get-Command echo))
$m.Name
$p = $m.Parameters.NoEnumerate
$clone = [System.Management.Automation.CommandMetadata]::new($m)
$p.Aliases.Add('Single')
"$($p.Aliases.Count) $($clone.Parameters.NoEnumerate.Aliases.Count) $($p.Aliases.Contains('Single'))"
$all = [System.Management.Automation.CommandMetadata]::new((Get-Command Write-Output),$true)
$all.Parameters.OutBuffer.Attributes | ForEach-Object { $_.GetType().Name }
(Get-Command Sort-Object).Parameters.Top.Attributes[1].MinRange
([System.Management.Automation.CommandMetadata]::new([Microsoft.PowerShell.Commands.WriteOutputCommand])).Name
try { $m = [System.Management.Automation.CommandMetadata]::new($null) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
$d = (Get-Command Write-Output).Parameters
"$($d.Keys.GetType().Name) $($d.Values.GetType().Name) $($d.Keys.Contains('inputobject'))"
"$($d.Keys.IsReadOnly) $($d.Keys -is [System.Collections.IList]) $($d.Keys[0].GetType().Name) $($null -eq $d.Keys[1])"
[object]::ReferenceEquals($d.Keys,$d.Keys)
[object]::ReferenceEquals($d.Values.SyncRoot,$d)
$a = $d.NoEnumerate.Aliases
$r = $a.SyncRoot
"$($r.GetType().Name) $([object]::ReferenceEquals($a,$r)) $([object]::ReferenceEquals($r,$a.SyncRoot))"
$r.Add('one')
$a.Insert(0,42)
$a[0] = 'two'
$a -join ','
try { $d.Keys.Add('invalid') } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { $d.Values.Clear() } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
$info = Get-Command Sort-Object
"$($info.Verb)|$($info.Noun)|$($info.ImplementingType.FullName)|$($info.DefaultParameterSet)"
([System.Management.Automation.CommandMetadata]::new($info.ImplementingType)).Name

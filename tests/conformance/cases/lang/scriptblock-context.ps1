# rpsh-conformance: level=c2
$v='caller'
function Read-Local { 'outer' }
$localFunctions=@{'Read-Local'={param($p) "fn:$p/$v"}}
$var=[psvariable]::new('v','injected')
$context=[Collections.Generic.List[PSVariable]]::new()
$context.Add($var)
$context.Add([psvariable]::new('this','owner'))
$context.Add([psvariable]::new('_',8))
$context.Add([psvariable]::new('input',(2,3)))
$script={param($p='default') Read-Local $p; "$_/$this/$($input -join '+')/$($args -join '+')/$($PSBoundParameters.Keys -join '+')"}
$result=$script.InvokeWithContext($localFunctions,$context,@('arg','extra'))
"$($result.GetType().Name)|$($result.Count)|$($result -join ',')|$($context.Count)|$($context[0].Name)"
$script.InvokeWithContext($localFunctions,$context)
Read-Local
"caller=$v; supplied=$($var.Value)"
$compiled={ $V; $v='local'; "$v/$((Get-Variable V).Value)" }
$compiled.InvokeWithContext($null,$var)
"after local=$($var.Value)"
$dynamic={ $v='assigned'; Set-Variable v 'cmdlet'; "$v/$((Get-Variable v).Value)" }
$dynamic.InvokeWithContext($null,$var)
"after cmdlet=$($var.Value)"
{ (Get-Variable v).Value='member'; $v }.InvokeWithContext(@{},$var)
"after member=$($var.Value)"
{param($v=20) "$v/$((Get-Variable v).Value)"}.InvokeWithContext(@{},$var,99)
try { { Read-Local 'first'; throw 'context failure' }.InvokeWithContext($localFunctions,$var) } catch {
    "$($_.FullyQualifiedErrorId)|$($_.Exception.InnerException.GetType().Name)|$($_.Exception.Message)"
}
Read-Local
$var.Name
{ $_; $this; $input }.InvokeWithContext(@{},@([psvariable]::new('_',4),[psvariable]::new('this',5),[psvariable]::new('input',6)))
$plain={ param($a) "a=$a;args=$($args -join ',')" }
$plain.InvokeWithContext($null,$null,1,2,3)
$plain.InvokeWithContext($null,$null,$null)
{ param($v) $v }.InvokeWithContext($null,@(),(,(1,2)))
foreach($run in { $plain.InvokeWithContext() },{ $plain.InvokeWithContext(@{}) },{ $plain.InvokeWithContext('wrong',$null) },{ $plain.InvokeWithContext(@{''={}},$null) },{ $plain.InvokeWithContext(@{7={}},$null) },{ $plain.InvokeWithContext(@{fn='invalid'},$null) },{ $plain.InvokeWithContext(@{},@($null)) },{ $plain.InvokeWithContext(@{},@($var,$var)) },{ $plain.InvokeWithContext(@{},'') }) {
    try { & $run } catch {
        $inner=$_.Exception.InnerException
        "$($_.FullyQualifiedErrorId)|$($_.Exception.GetType().Name)|$($inner -eq $null ? '' : $inner.GetType().Name)|$($_.Exception.Message)"
    }
}

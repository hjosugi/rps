# rpsh-conformance: level=c2 LANG=C
$cmd=Get-Command Switch-Process
"$($cmd.Name)|$($cmd.CommandType)|$($cmd.GetType().FullName)|$($cmd.Source)|$($cmd.Visibility)|$($cmd.Options)"
$cmd -is [System.Management.Automation.CommandInfo]
$cmd.Definition.Trim()
$func=Get-Command exec
"$($func.Name)|$($func.CommandType)|$($func.GetType().FullName)|$($func.Source)|$($func.Visibility)|$($func.Options)"
$func -is [System.Management.Automation.CommandInfo]
$func.Definition.Trim()
(Get-Item Function:exec).Definition -ceq $func.Definition
Switch-Process
exec
$env:RPSH_EXEC_EXPECTED_PID=[string]$PID
$command='/bin/sh','-c','test "$$" = "$RPSH_EXEC_EXPECTED_PID" && echo same-pid; printf "<%s>\n" "$0" "$@"; exit 23','child','','--flag','a b','ü'
Microsoft.PowerShell.Core\Switch-Process -WithCommand $command
'unreachable'

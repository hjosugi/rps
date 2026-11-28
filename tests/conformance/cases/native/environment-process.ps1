# rpsh-conformance: level=c2
$env:RPSH_ENV_Case = 42
$env:RPSH_ENV_CASE = 'upper'
$ENV:RPSH_ENV_Case.GetType().Name
[Environment]::GetEnvironmentVariable('RPSH_ENV_Case')
[Environment]::GetEnvironmentVariable('RPSH_ENV_CASE', 'Process')
$null -eq [Environment]::GetEnvironmentVariable('rpsh_env_case')
Test-Path Env:rpsh_env_case
Test-Path Env:RPSH_ENV_Case
(Get-Item Env:RPSH_ENV_Case).Value
$env:RPSH_ENV_Case = ''
$null -eq $env:RPSH_ENV_Case
Test-Path Env:RPSH_ENV_Case
sh -c 'test "${RPSH_ENV_Case+x}" = x && test -z "$RPSH_ENV_Case" && echo present-empty'
$env:RPSH_ENV_Case = $null
Test-Path Env:RPSH_ENV_Case
$null -eq [Environment]::GetEnvironmentVariable('RPSH_ENV_Case')
sh -c 'test -z "${RPSH_ENV_Case+x}" && echo removed'
[Environment]::SetEnvironmentVariable('RPSH_ENV_Case',$null)
Test-Path Env:RPSH_ENV_Case
[Environment]::SetEnvironmentVariable('RPSH_ENV_Case',[NullString]::Value)
Test-Path Env:RPSH_ENV_Case
function Set-TestEnvironment { [Environment]::SetEnvironmentVariable('RPSH_ENV_Case','nested',[EnvironmentVariableTarget]::Process) }
Set-TestEnvironment
$env:RPSH_ENV_Case
sh -c 'printf "%s/%s\n" "$RPSH_ENV_Case" "$RPSH_ENV_CASE"'
$table = [Environment]::GetEnvironmentVariables()
$table.GetType().FullName
$table['RPSH_ENV_Case']
$table['RPSH_ENV_CASE']
$table.ContainsKey('rpsh_env_case')
$table['RPSH_ENV_Case'] = 'snapshot'
$table['RPSH_ENV_Case']
$env:RPSH_ENV_Case
[Environment]::GetEnvironmentVariables('User').Count
[Environment]::SetEnvironmentVariable('RPSH_ENV_Case','other','Machine')
$null -eq [Environment]::GetEnvironmentVariable('RPSH_ENV_Case','Machine')
$env:RPSH_ENV_Case
try { [Environment]::SetEnvironmentVariable('A=B','value') } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { [Environment]::GetEnvironmentVariable('RPSH_ENV_Case', $null) } catch { $_.FullyQualifiedErrorId }
try { [Environment]::GetEnvironmentVariable('RPSH_ENV_Case', -1) } catch { $_.FullyQualifiedErrorId }
$zone = $env:TZ
Remove-Item Env:TZ
$null -eq $env:TZ
Test-Path Env:TZ
sh -c 'test -z "${TZ+x}" && echo inherited-removed'
$env:TZ = $zone
($env:PATH -split ':')[0] -ceq $PSHOME
($env:PSModulePath -split ':') -ccontains "$PSHOME/Modules"
($env:PSModulePath -split ':') -ccontains "$HOME/.local/share/powershell/Modules"
sh -c 'test "$1" = "$PSModulePath" && echo module-path' sh $env:PSModulePath
$oldPath = $env:PATH
$caseDir = mktemp -d
try {
    sh -c 'mkdir "$1/first" "$1/second"; for dir in first second; do printf "#!/bin/sh\necho %s\n" "$dir" > "$1/$dir/rpsh-env-probe"; chmod +x "$1/$dir/rpsh-env-probe"; done' sh $caseDir
    $env:PATH = "$caseDir/first:$oldPath"
    rpsh-env-probe
    [Environment]::SetEnvironmentVariable('PATH', "$caseDir/second:$oldPath")
    rpsh-env-probe
} finally {
    $env:PATH = $oldPath
    Remove-Item -LiteralPath $caseDir -Recurse -Force
}

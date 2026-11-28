# rpsh-conformance: level=c2
sh -c 'exit 4' && 'skipped' || 'fallback' && 'after fallback'
sh -c 'exit 0' || 'skipped' && 'after success'
$false && 'false value succeeded'
$true || 'skipped'
sh -c 'printf left; exit 2' |
    Write-Output && 'skipped' || Write-Output 'right'
Write-Error quiet -ErrorAction SilentlyContinue || 'caught'
sh -c 'exit 7' ||
    Write-Output $?
"last=$?|$LASTEXITCODE"
$all = Write-Output one && Write-Output two || Write-Output skip
$all -join ','
$head, $tail = sh -c 'printf first; exit 1' || Write-Output second third
"$head|$($tail -join ',')"
$all = (1 && 2) || 'skipped'
$all -join ','
function Double-Input { process { $_ * 2 } }
$all = 1..2 | Double-Input && 'finish'
$all -join ','
if ($false && $true) { 'array condition' }
function Stop-Test { throw 'terminating' }
try { 'inside' && Stop-Test || 'skipped' } catch { 'catch' }

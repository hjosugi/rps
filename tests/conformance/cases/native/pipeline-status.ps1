# rpsh-conformance: level=c2
"entry=$?"
sh -c 'exit 3' | Write-Output
"first=$?|$LASTEXITCODE"
Write-Output 'input' | sh -c 'cat >/dev/null; exit 2'
"last=$?|$LASTEXITCODE"
sh -c 'exit 3' | sh -c 'exit 0'
"both=$?|$LASTEXITCODE"
sh -c 'exit 4'
Write-Output $?
"output=$?|$LASTEXITCODE"
function Invoke-UnsuccessfulNative { sh -c 'exit 5' }
Invoke-UnsuccessfulNative
"function=$?|$LASTEXITCODE"
Write-Error quiet -ErrorAction SilentlyContinue | Write-Output
"error=$?"
(sh -c 'exit 6')
"group=$?|$LASTEXITCODE"
$result = (sh -c 'exit 6')
"assigned group=$?"
$a, $b = (sh -c 'exit 6')
"multiple group=$?"
$result = sh -c 'printf result; exit 7'
"assignment=$?|$LASTEXITCODE|$result"
sh -c 'kill -TERM $$'
"signal=$?|$LASTEXITCODE"
sh -c 'exit 8'

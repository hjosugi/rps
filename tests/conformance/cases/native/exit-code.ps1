# rpsh-conformance: level=c2
sh -c 'exit 3'
$?
$LASTEXITCODE
sh -c 'exit 0'
$?
$LASTEXITCODE

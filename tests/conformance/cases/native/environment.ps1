# rpsh-conformance: level=c2
$env:RPSH_CASE = 'set in script'
sh -c 'echo "$RPSH_CASE"'
$env:RPSH_CASE = $null
sh -c 'echo "[$RPSH_CASE]"'

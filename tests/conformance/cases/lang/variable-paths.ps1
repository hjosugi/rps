# rpsh-conformance: level=c2
# Variable paths: braces, scopes, environment drive and the special variables.
$x = 1
${a b} = 2
$script:y = 3
$env:RPSH_VARIABLE_TEST = 'v'
$x
${a b}
$y
$script:y
$env:RPSH_VARIABLE_TEST
$null
$true
$false
$?
$_

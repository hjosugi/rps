# rpsh-conformance: level=c2
if ($args.Count) {
    "invoked=$($args.GetType().Name)/$($args.Count)/$($args -join '|')"
    "file=$(Split-Path -Leaf $PSCommandPath)/$(Split-Path -Leaf $PSScriptRoot)"
    return
}
"root=$($args.GetType().Name)/$($args.Count)/$(Test-Path Variable:args)"
& $PSCommandPath first 'two words'
"restored=$($args.Count)/$(Split-Path -Leaf $PSCommandPath)"

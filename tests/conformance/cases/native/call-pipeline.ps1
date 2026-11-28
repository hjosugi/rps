# rpsh-conformance: level=c2 LANG=C
$tool='./fixtures/argv-dump'
$values=@('a b','',7)
& $tool -switch:2 @values
& ($tool) --% $literal %RPSH_NOT_SET% two words
$filter='grep'
& 'printf' '%s\n' 'red' 'blue' | & $filter -F blue | & 'cat'
& 'printf' '\377\0\376' | & 'wc' -c | ForEach-Object { $_.Trim() }
$application=Get-Command './fixtures/argv-dump'
& $application 'from info'
& 'sh' -c 'exit 3' || & 'printf' '%s\n' recovered
$output=& $tool 'captured value'
$output
$order=[System.Collections.ArrayList]::new()
function StageName($id,$name) { $null=$order.Add("name$id"); $name }
function StageArg($id,$value) { $null=$order.Add("arg$id"); $value }
& (StageName 1 'printf') (StageArg 1 '%s\n') 'hello' | & (StageName 2 grep) (StageArg 2 hello) | & (StageName 3 cat)
$order -join '/'

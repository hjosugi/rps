# rpsh-conformance: level=c2 LANG=C
$x='expanded'
$env:RPSH_STOP='two words'
$env:RPSH_NESTED='%RPSH_STOP%'
$env:RPSH_NONE=$null
foreach($style in 'Standard','Legacy','Windows') {
    $PSNativeCommandArgumentPassing=$style
    "$style"
    ./fixtures/argv-dump --% $x %RPSH_STOP%   "a b" 'c d'
    ./fixtures/argv-dump prefix --% %RPSH_NESTED% %%RPSH_STOP% %RPSH_NONE%RPSH_STOP% %rpsh_stop%
    ./fixtures/argv-dump --% foo; #comment 2>file $() () @array ${var} & &
    ./fixtures/argv-dump --% "within|quote" and "&&" | ForEach-Object { "pipe=$_" }
    ./fixtures/argv-dump --% first && ./fixtures/argv-dump after
    ./fixtures/argv-dump --% first || ./fixtures/argv-dump skipped
    ./fixtures/argv-dump --%
    ./fixtures/argv-dump --% starting	inside	one	last
    ./fixtures/argv-dump --% "trailing\" quote" extra
    ./fixtures/argv-dump '--%' $x "one two" '%RPSH_STOP%' '*.txt'
    $marker='--%'
    ./fixtures/argv-dump $marker '%RPSH_STOP%'
    ./fixtures/argv-dump @('--%','%RPSH_STOP%') end
    ./fixtures/argv-dump -p: --% $x '%RPSH_STOP%'
    ./fixtures/argv-dump --%x 'ordinary'
}
function Show-Verbatim { foreach($v in $args) { "[$v]" } }
Show-Verbatim --% $x %RPSH_STOP% "a b" #literal

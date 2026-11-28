# rpsh-conformance: level=c2 LANG=C
$x='A Z'; $y=3; $n=2; $h=@{x='hello'}
foreach ($style in 'Standard','Legacy','Windows') {
    $PSNativeCommandArgumentPassing=$style
    $style
    ./fixtures/argv-dump a"b c"d a"b""c"d a'b c'd a'it''s'b
    ./fixtures/argv-dump "a"b 'a'b "a"$x 1"ab"c -"name" -x="a b"
    ./fixtures/argv-dump -x=$x -x=$h.x -x$z:yes --flag:$x -opt=$x
    ./fixtures/argv-dump a$(1+2)b $(1+2)b $x"b c"z a'$x'"$x" @x"b d"c
    ./fixtures/argv-dump x"*.txt" a£“ü ö”z a$x.y 'a b'
    ./fixtures/argv-dump a=b=c 42foo -01 -9 -
    ./fixtures/argv-dump -foo.bar -f[a] -f! a#b x>z x<y a$ a$()b
    ./fixtures/argv-dump a$(Write-Output 'x y ')b "s$(1+2)" a$(1;2)b
    ./fixtures/argv-dump a"b$(1+2)c"d a${h}hello a"foo"z'bar' a"abc"`"d
    ./fixtures/argv-dump a$(Write-Output ""foo"")b
    ./fixtures/argv-dump a$(Write-Output `"foo`")b
    ./fixtures/argv-dump a"b$(Write-Output ""foo"")z"c
    ./fixtures/argv-dump g“left”“right”end a’it’’s‘b
}

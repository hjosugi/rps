# rpsh-conformance: level=c2 LANG=C
$x='xx'; $v='a b'; $n=$null
foreach ($mode in 'Standard','Legacy','Windows') {
    $PSNativeCommandArgumentPassing=$mode
    $mode
    ./fixtures/argv-dump 1,2 1, 2 1 ,2 1 , 2
    ./fixtures/argv-dump aa,bb aa, bb aa ,bb aa , bb
    ./fixtures/argv-dump "a","b" 'a','b' "$x",z
    ./fixtures/argv-dump 1,$x 002,0xF (1,2) @(1,2) (1+2),abc
    ./fixtures/argv-dump $v,'bb cc' $x,foo 'aa bb','cc dd'
    ./fixtures/argv-dump $null,'a' '',$x $x,$null
    ./fixtures/argv-dump -n:1,2 -k: 'a b',foo -f x,y -z a, b
    ./fixtures/argv-dump -k com:port=\devbox\pipe\debug,pipe,resets=0,reconnect
    ./fixtures/argv-dump /arg1 /c:"a string"
    ./fixtures/argv-dump -F: '{print $1}'
    ./fixtures/argv-dump before $null,(1+2),('x','y') , *.no-such-file, 0xA after
    ./fixtures/argv-dump 1,2,3 1, 2,3 1 ,2,3 1 , 2,3
    ./fixtures/argv-dump first 1,2 3,4 second
    ./fixtures/argv-dump $x,'a b',bb
    ./fixtures/argv-dump 1`
    ,2
}
function Show-BoundArray($words) {
    $words.GetType().Name
    foreach ($v in $words) { "$($v.GetType().Name)|$v" }
    "rest=$($args -join '|')"
}
Show-BoundArray 001,0xF tail
Show-BoundArray -words ('ab','cd'),$x
$dir=mktemp -d
try {
    cp fixtures/argv-dump "$dir/run.CmD"
    Push-Location $dir
    try {
        foreach($mode in 'Standard','Legacy','Windows') {
            $PSNativeCommandArgumentPassing=$mode
            ./run.CmD 1, 2 'a b',x -n:1 , 2
        }
    } finally { Pop-Location }
} finally { Remove-Item $dir -Recurse -Force }

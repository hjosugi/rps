# rpsh-conformance: level=c2 LANG=C
function Inspect-Num($n) {
    $n.GetType().Name; $n; $n.ToString(); "expand=$n"; [string]$n
    ($n -is [int]); [int]$n; [bool]$n; $n+2; 30-$n; $n*2; $n -gt 1
    "$($n+1)|$(-$n)"; $n -band 3; $n -shl 1
    "$($n..($n+2))"; ('v'+$n); '{0:D5}' -f $n
    $n | ConvertTo-Json -Compress
}
function Pass-Nums { ./fixtures/argv-dump @args }
foreach($mode in 'Standard','Legacy','Windows') {
    $PSNativeCommandArgumentPassing=$mode
    $mode
    ./fixtures/argv-dump 001 0xF 0b11 10mb 2.0 2.50d 4L 0xFFuy 1e2 -01
    ./fixtures/argv-dump -n:0xF -n: 003 -n 1KB
    ./fixtures/argv-dump (0x10) (1e2) ([int]0x10) $(0001)
    Pass-Nums 001 0xF 1kb 1.00 1e3
}
Inspect-Num 0x10
Inspect-Num 000
function Show-Number($v) {
    "$v|$($v.GetType().Name)|$($v.ToString())"
    '{0:F5}' -f $v
    ./fixtures/argv-dump $v
}
Show-Number 1.2345678912345678
Show-Number 1.234567891234568
Show-Number 1E+16
Show-Number 1E+17
Show-Number 1E-06
Show-Number 1e-6
function Inspect-Number($n,$m) {
    $n.GetType().FullName; "bare=$n|sub=$($n)|join=$(($n,$m) -join '|')"
    [string]$n; ([string[]]@($n,$m)) -join '|'; ($n.ToString()); $n + 2; $n -eq 16
    $store=@($n,$m); "array=$store"; $store -join '|'; $n | ConvertTo-Json -Compress
    ./fixtures/argv-dump $n ($n) $store ([string]$n) ([int]$n)
}
Inspect-Number 0x10 002
$x=Write-Output 0x10; $y=$x; ./fixtures/argv-dump $x $y

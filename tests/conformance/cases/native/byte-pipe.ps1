# rpsh-conformance: level=c2
$path = mktemp
try {
    sh -c 'printf "\377\000ABC\rZ\200\n"' | cat | cat > $path
    (Get-FileHash -LiteralPath $path).Hash
    sh -c 'dd if=/dev/zero bs=16384 count=32 2>/dev/null' | cat | wc -c | ForEach-Object { [int]$_ }
    sh -c 'printf no-final-newline' | cat
    '!'
    $lines = sh -c 'printf "a\rb\r\n\nc"' | cat
    $lines.Count
    $lines -join '|'
    $large = 'z' * 131072
    $echoed = $large | cat
    $echoed.Length
    $large | cat | sh -c 'dd bs=1 count=1 2>/dev/null'
    '!'
    sh -c 'exit 3' | cat | sh -c 'exit 0'
    "$?|$LASTEXITCODE"
} finally {
    Remove-Item $path
}

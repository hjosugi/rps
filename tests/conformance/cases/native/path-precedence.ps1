# rpsh-conformance: level=c2
$prior=$env:PATH
$tmp=mktemp -d
try {
    sh -c 'mkdir "$1/a" "$1/b"; ln -s /bin/true "$1/b/rpsh-lookup-check"' sh $tmp
    $env:PATH="$tmp/a:$tmp/b:$prior"
    rpsh-lookup-check
    $LASTEXITCODE
    sh -c 'ln -s /bin/false "$1/a/rpsh-lookup-check"' sh $tmp
    rpsh-lookup-check
    $LASTEXITCODE
    sh -c 'unlink "$1/a/rpsh-lookup-check"' sh $tmp
    rpsh-lookup-check
    $LASTEXITCODE
} finally {
    $env:PATH=$prior
    Remove-Item -LiteralPath $tmp -Recurse -Force
}

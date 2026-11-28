# rpsh-conformance: level=c2 LANG=C
$prior=$env:PATH
$tmp=mktemp -d
try {
    sh -c 'mkdir "$1/a" "$1/b"; printf "#!/bin/sh\nexit 0\n" > "$1/b/rpsh-lookup-check"; chmod +x "$1/b/rpsh-lookup-check"' sh $tmp
    $env:PATH="$tmp/a:$tmp/b:$prior"
    rpsh-lookup-check
    $LASTEXITCODE
    sh -c 'printf "#!/bin/sh\nexit 1\n" > "$1/a/rpsh-lookup-check"; chmod +x "$1/a/rpsh-lookup-check"' sh $tmp
    rpsh-lookup-check
    $LASTEXITCODE
    sh -c 'unlink "$1/a/rpsh-lookup-check"' sh $tmp
    rpsh-lookup-check
    $LASTEXITCODE
} finally {
    $env:PATH=$prior
    Remove-Item -LiteralPath $tmp -Recurse -Force
}

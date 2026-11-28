# rpsh-conformance: level=c2 LANG=C
$dir = mktemp -d
$prior = $env:PATH
try {
    sh -c 'mkdir "$1/a" "$1/b"; for sub in a b; do printf "#!/bin/sh\necho %s\n" "$sub" > "$1/$sub/rpsh-path-test"; chmod +x "$1/$sub/rpsh-path-test"; done; printf "exit 17\n" > "$1/a/rpsh-plain-test"; chmod +x "$1/a/rpsh-plain-test"' sh $dir
    $env:PATH = " $dir/a :$dir/b:$dir/a:$prior"
    rpsh-path-test
    $app = Get-Command rpsh-path-test
    $app.Name
    $app.GetType().FullName
    $app.CommandType
    $app.Version.ToString()
    $app.Path -ceq "$dir/a/rpsh-path-test"
    $app.Source -ceq $app.Definition
    (Get-Command rpsh-path-test -All | ForEach-Object { $_.Path.Replace($dir,'temp') }) -join '|'
    $env:PATH = "$dir/b:$dir/a:$prior"
    rpsh-path-test
    $next = Get-Command rpsh-path-test -CommandType Application
    $next.Count
    $next[0].Path -ceq "$dir/b/rpsh-path-test"
    try { rpsh-plain-test } catch {
        $_.FullyQualifiedErrorId
        $_.Exception.GetType().FullName
        $_.Exception.Message -like '*Exec format error*'
    }
    sh -c 'printf "#!%s/missing-interpreter\n" "$1" > "$1/a/rpsh-bad-interpreter"; chmod +x "$1/a/rpsh-bad-interpreter"' sh $dir
    try { rpsh-bad-interpreter } catch {
        $_.FullyQualifiedErrorId
        $_.Exception.Message -like "*rpsh-bad-interpreter*"
    }
    Push-Location $dir
    try {
        $env:PATH = "./a:$prior"
        rpsh-path-test
        $env:PATH = ".:$prior"
        Push-Location ./b
        try { rpsh-path-test } finally { Pop-Location }
    } finally { Pop-Location }
    sh -c 'case "$0" in /*/sh) echo absolute ;; *) echo relative ;; esac'
    $env:PATH = ''
    try { rpsh-path-test } catch { $_.FullyQualifiedErrorId }
} finally {
    $env:PATH = $prior
    Remove-Item -LiteralPath $dir -Recurse -Force
}
Get-Command sh

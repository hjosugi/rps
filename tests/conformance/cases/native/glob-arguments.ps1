# rpsh-conformance: level=c2
$caseDir = mktemp -d
$tildeDir = mktemp -d "$HOME/rpsh-native-glob.XXXXXXXX"
Push-Location $caseDir
try {
    sh -c 'mkdir -p aDir/sub Bdir/sub "space dir" .hidden; touch alpha.txt B.TXT É.txt _.txt 1.txt .hidden.txt "space file.txt" aDir/First.txt aDir/y.txt Bdir/only.txt aDir/sub/x.txt Bdir/sub/z.txt "space dir/file.txt" .hidden/visible.txt'
    /usr/bin/printf '<%s>\n' *
    /usr/bin/printf '<%s>\n' [aB]*.txt
    /usr/bin/printf '<%s>\n' B.?xt
    /usr/bin/printf '<%s>\n' *dir/sub/*.txt
    /usr/bin/printf '<%s>\n' *Dir/SUB
    /usr/bin/printf '<%s>\n' ./a*/../*.txt
    /usr/bin/printf '<%s>\n' missingDir/*.txt
    /usr/bin/printf '<%s>\n' .* .hidden/* */.* none-[0-9]?.zzz
    /usr/bin/printf '<%s>\n' '*.txt' "*.txt" ('*.txt')
    $pattern = 'aDir/*.txt'
    /usr/bin/printf '<%s>\n' $pattern "$pattern" -flag $pattern -flag '*.txt'
    /usr/bin/printf '<%s>\n' -flag aDir/*.txt
    $parentDir = 'aDir'
    /usr/bin/printf '<%s>\n' $parentDir/*.txt ${parentDir}/sub/*.txt
    /usr/bin/printf '<%s>\n' aDir/`*.txt aDir\*.txt
    sh -c 'touch "aDir/a*xyz"'
    $escaped = 'aDir/a`*x*'
    /usr/bin/printf '<%s>\n' $escaped 'aDir/a[*]x*' ('aDir/a[*]x*')
    $words = @('B.?xt','no-*.txt','space dir/*.txt')
    /usr/bin/printf '<%s>\n' $words
    Write-Output *.txt
    sh -c 'test "$1" = "$HOME" && test "$2" = "$HOME/missing" && test "$3" = "~" && test "$4" = "~/missing-*" && echo tilde' sh ~ ~/missing '~' ~/missing-*
    sh -c 'touch "$1/a.log"' sh $tildeDir
    $leaf = sh -c 'basename "$1"' sh $tildeDir
    $tildePattern = "~/$leaf/*.log"
    sh -c 'test "$1" = "$2/a.log" && echo tilde-wildcard' sh $tildePattern $tildeDir
} finally {
    Pop-Location
    Remove-Item -LiteralPath $caseDir -Recurse -Force
    Remove-Item -LiteralPath $tildeDir -Recurse -Force
}

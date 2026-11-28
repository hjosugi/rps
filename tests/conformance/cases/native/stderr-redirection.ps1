# rpsh-conformance: level=c2 LANG=C
$dir=mktemp -d
function Read-Hex($file) {
    sh -c 'od -An -v -tx1 "$1" | tr -d " \n"; printf "\n"' sh $file
}
try {
    $Error.Clear()
    sh -c 'printf "\000\377O"; printf "\000\377E\r\nZ" >&2' > "$dir/out" 2> "$dir/err"
    Read-Hex "$dir/out"
    Read-Hex "$dir/err"
    sh -c 'printf "\rA\n\nB" >&2; exit 1' 2>> "$dir/err"
    "$?|$LASTEXITCODE|$($Error.Count)"
    Read-Hex "$dir/err"
    sh -c 'printf ignored >&2; printf stdout' 2>$null | sh -c 'cat; echo'
    $captured=sh -c 'printf "object\nline"; printf another >&2' 2> "$dir/err"
    $captured -join '|'
    Read-Hex "$dir/err"
    sh -c 'printf "one" >&2; printf "\377\000"' 2> "$dir/one" | sh -c 'printf "two" >&2; cat' 2> "$dir/two" > "$dir/raw"
    Read-Hex "$dir/one"
    Read-Hex "$dir/two"
    Read-Hex "$dir/raw"
    $PSApplicationOutputEncoding=[Text.Encoding]::Unicode
    sh -c 'printf "\377\376A\000\015\000\012\000B\000" >&2' 2> "$dir/err"
    $PSApplicationOutputEncoding=[Text.Encoding]::UTF8
    Read-Hex "$dir/err"
    sh -c 'dd if=/dev/zero bs=8192 count=40 2>/dev/null | tr "\000" e >&2; printf "x" >&2; printf "done\n"' 2> "$dir/long"
    (Get-Content "$dir/long").Length
    $PSNativeCommandUseErrorActionPreference=$true
    foreach($action in 'Continue','SilentlyContinue','Ignore','Stop') {
        $ErrorActionPreference=$action
        $Error.Clear()
        try {
            sh -c 'printf "err\r\ntwo" >&2; exit 7' 2> "$dir/err"
            "$action|$?|$LASTEXITCODE|$($Error.Count)"
        } catch { "catch|$action|$?|$LASTEXITCODE|$($Error.Count)|$($_.FullyQualifiedErrorId)" }
        (Get-Content "$dir/err") -join '|'
    }
    $ErrorActionPreference='Continue'
    $Error.Clear()
    sh -c 'printf ignored >&2; exit 7' 2>$null
    "$?|$LASTEXITCODE|$($Error.Count)"
} finally { Remove-Item $dir -Recurse -Force }

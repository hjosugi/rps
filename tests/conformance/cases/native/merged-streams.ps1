# rpsh-conformance: level=c2 LANG=C
$dir=mktemp -d
function Read-Hex($file) {
    sh -c 'od -An -v -tx1 "$1" | tr -d " \n"; printf "\n"' sh $file
}
try {
    $Error.Clear()
    $ErrorActionPreference='Stop'
    $v=sh -c 'printf "out\r\n"; sleep 0.15; printf "one\r\ntwo\n\nthree" >&2' 2>&1
    "$?|$LASTEXITCODE|$($Error.Count)|$($v.Count)"
    foreach($i in $v) {
        "$($i.GetType().FullName)|$i"
        if($i -is [System.Management.Automation.ErrorRecord]) {
            "$($i.FullyQualifiedErrorId)|$($i.Exception.GetType().FullName)|$($i.Exception.Message)"
            "$($i.CategoryInfo)|$($i.CategoryInfo.TargetName)|$($i.TargetObject -eq $null)"
        }
    }
    $v=sh -c 'echo 12345678901234567890123456789012345678901234567890 >&2' *>&1
    "$($v.CategoryInfo)|$($v.TargetObject)"
    sh -c 'printf "bad\377\n" >&2' 2>&1 | sh -c 'od -An -v -tx1 | tr -d " \n"; echo'
    $v=sh -c 'printf "input\n"' | sh -c 'cat; printf "error\n" >&2' 2>&1
    ($v | Sort-Object) -join '|'
    sh -c 'printf "\377o"; printf "long\r\nshort" >&2' *> $null
    "discard|$?|$($Error.Count)"
    sh -c 'printf "\377A\r\n"; sleep 0.15; printf "e\377\nz" >&2' *> "$dir/all"
    sh -c 'printf "\n" >&2' *>> "$dir/all"
    Read-Hex "$dir/all"
    sh -c 'printf "e\377\n" >&2; sleep 0.15; printf "o\377"' 2>&1 > "$dir/both"
    Read-Hex "$dir/both"
    sh -c 'printf "o\377\n"; printf "e\377\n" >&2' *> "$dir/all" 2> "$dir/err"
    Read-Hex "$dir/all"
    Read-Hex "$dir/err"
    sh -c 'printf "o\377\n"; printf "e\377\n" >&2' *> "$dir/all" > "$dir/raw"
    Read-Hex "$dir/all"
    Read-Hex "$dir/raw"
    sh -c 'printf "one\n" >&2; printf "two\n" >&2' *>&1 2> "$dir/err"
    Read-Hex "$dir/err"
    sh -c 'echo x; sleep 0.15; echo y >&2' 2>&1
    sh -c 'i=0; while [ "$i" -lt 8000 ]; do echo "stdout-line-$i"; echo "stderr-line-$i" >&2; i=$((i+1)); done' *> "$dir/many"
    awk '-F-' '/^stdout-line-/ {if ($3 != o++) bad++} /^stderr-line-/ {if ($3 != e++) bad++} END {print o,e,bad+0}' "$dir/many"
    $PSApplicationOutputEncoding=[Text.Encoding]::Unicode
    $v=sh -c 'printf "\101\000\015\000\012\000\102\000" >&2' 2>&1
    $PSApplicationOutputEncoding=[Text.Encoding]::UTF8
    $v -join '|'
    $PSNativeCommandUseErrorActionPreference=$true
    foreach($action in 'Continue','SilentlyContinue','Ignore','Stop') {
        $ErrorActionPreference=$action
        $Error.Clear()
        try {
            $v=sh -c 'printf "ordinary\n" >&2; exit 7' 2>&1
            "$action|$?|$LASTEXITCODE|$($Error.Count)|$($v.Count)"
            foreach($i in $v) { "$($i.FullyQualifiedErrorId)|$i" }
        } catch { "catch|$action|$?|$LASTEXITCODE|$($Error.Count)|$($_.FullyQualifiedErrorId)" }
    }
    $ErrorActionPreference='Continue'
    $v=sh -c 'echo err >&2; exit 3' 2>&1
    $v | Out-String
    $v | sh -c 'cat'
    $v
    sh -c 'echo line >&2; exit 3' *> "$dir/all"
    Get-Content "$dir/all"
} finally { Remove-Item $dir -Recurse -Force }

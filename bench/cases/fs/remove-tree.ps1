# Remove a 2,000-file tree.
$root = Join-Path ([System.IO.Path]::GetTempPath()) "rpsh-bench-remove-$PID"
for ($d = 1; $d -le 40; $d++) {
    $dir = Join-Path $root "dir$d"
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    for ($f = 1; $f -le 50; $f++) {
        [System.IO.File]::WriteAllText((Join-Path $dir "file$f.txt"), "$d $f")
    }
}
(Get-ChildItem -Path $root -Recurse -File).Count
Remove-Item -Path $root -Recurse -Force
Test-Path $root

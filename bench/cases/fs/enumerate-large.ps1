# Create and enumerate 3,200 files in 80 directories.
$root = Join-Path ([System.IO.Path]::GetTempPath()) "rpsh-bench-enum-large-$PID"
New-Item -ItemType Directory -Path $root -Force | Out-Null
for ($d = 1; $d -le 80; $d++) {
    $dir = Join-Path $root "dir$d"
    New-Item -ItemType Directory -Path $dir | Out-Null
    for ($f = 1; $f -le 40; $f++) {
        [System.IO.File]::WriteAllText((Join-Path $dir "file$f.txt"), "$d $f")
    }
}
(Get-ChildItem -Path $root -Recurse -File).Count
(Get-ChildItem -Path $root -Recurse -Filter 'file1*.txt').Count
Remove-Item -Path $root -Recurse -Force
Test-Path $root

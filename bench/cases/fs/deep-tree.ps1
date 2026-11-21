# A 30-level directory tree: path handling and recursive enumeration.
$root = Join-Path ([System.IO.Path]::GetTempPath()) "rpsh-bench-deep-$PID"
$dir = $root
for ($i = 1; $i -le 30; $i++) {
    $dir = Join-Path $dir "level$i"
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $dir 'file.txt'), "$i")
}
(Get-ChildItem -Path $root -Recurse -File).Count
(Get-ChildItem -Path $root -Recurse -Directory).Count
Remove-Item -Path $root -Recurse -Force
Test-Path $root

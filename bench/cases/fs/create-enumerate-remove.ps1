# Item cmdlets through the FileSystem provider: create, list, read, delete.
$root = Join-Path ([System.IO.Path]::GetTempPath()) "rpsh-bench-$PID"
New-Item -ItemType Directory -Path $root | Out-Null
foreach ($d in 1..10) {
    $dir = New-Item -ItemType Directory -Path (Join-Path $root "dir$d")
    foreach ($f in 1..50) { Set-Content -Path (Join-Path $dir "file$f.txt") -Value "content $d $f" }
}
(Get-ChildItem -Path $root -Recurse -File).Count
(Get-ChildItem -Path $root -Recurse -Filter 'file1*.txt' | Measure-Object -Property Length -Sum).Sum
(Get-Content -Path (Join-Path $root 'dir3' 'file7.txt'))
Remove-Item -Path $root -Recurse -Force
Test-Path $root

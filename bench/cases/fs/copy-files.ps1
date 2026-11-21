# Copy 1,000 small files and read one back.
$root = Join-Path ([System.IO.Path]::GetTempPath()) "rpsh-bench-copy-$PID"
$src = Join-Path $root 'src'
$dst = Join-Path $root 'dst'
New-Item -ItemType Directory -Path $src -Force | Out-Null
New-Item -ItemType Directory -Path $dst -Force | Out-Null
for ($i = 1; $i -le 1000; $i++) {
    [System.IO.File]::WriteAllText((Join-Path $src "file$i.txt"), "content $i")
}
Copy-Item -Path (Join-Path $src '*') -Destination $dst
(Get-ChildItem -Path $dst -File).Count
Get-Content -Path (Join-Path $dst 'file500.txt')
Remove-Item -Path $root -Recurse -Force

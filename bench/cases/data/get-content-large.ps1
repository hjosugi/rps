# Get-Content on a 50k-line file written by the case itself.
$path = Join-Path ([System.IO.Path]::GetTempPath()) "rpsh-bench-getcontent-$PID.txt"
$writer = [System.IO.File]::CreateText($path)
for ($i = 1; $i -le 50000; $i++) { $writer.WriteLine("line $i value $($i % 97)") }
$writer.Dispose()
$lines = Get-Content -Path $path
$lines.Count
($lines | Select-String -Pattern 'value 0' -SimpleMatch).Count
Remove-Item -Path $path

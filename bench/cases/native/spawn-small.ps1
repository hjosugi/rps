# Starting a native command 10 times.
$failures = 0
foreach ($i in 1..10) { true; if ($LASTEXITCODE -ne 0) { $failures++ } }
$failures

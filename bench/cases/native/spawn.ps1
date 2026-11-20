# Starting a native command many times: process creation and exit-code handling.
$failures = 0
foreach ($i in 1..100) { true; if ($LASTEXITCODE -ne 0) { $failures++ } }
$failures

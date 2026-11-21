# rpsh-conformance: level=c2
# Each of PowerShell's six streams, tagged and in the order it was written.
# Redirection turns a record into an object, so the golden shows the record
# type and its text; C2 also compares whatever actually reaches stderr.
$VerbosePreference = 'Continue'
$DebugPreference = 'Continue'
function Show([string] $tag, $value) { "${tag}:$value" }
Show 'output' (Write-Output 'out')
Show 'warning' (Write-Warning 'warn' 3>&1)
Show 'verbose' (Write-Verbose 'verb' 4>&1)
Show 'debug' (Write-Debug 'dbg' 5>&1)
Show 'information' (Write-Information 'info' 6>&1)
Show 'error' (Write-Error 'err' -ErrorAction Continue 2>&1)

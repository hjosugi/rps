# rpsh-conformance: level=c2
function Read-Err {
    end { throw 'primary' }
    clean { throw 'secondary' }
}
try { Read-Err } catch { "caught=$($_.Exception.Message)" }
foreach ($e in $Error) { "error=$($e.Exception.Message)" }
function Read-OK {
    end { 'ok' }
    clean { throw 'cleanup' }
}
try { Read-OK } catch { "unexpected=$($_.Exception.Message)" }
'continued'

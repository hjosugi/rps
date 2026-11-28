# rpsh-conformance: level=c2
function Read-PhaseOrder {
    clean { $script:events += "c($_/$input)"; 'discard' }
    end { $script:events += "e($_/$input)" }
    process { $script:events += "p($_/$input)" }
    begin { $script:events += "b($_/$input)" }
    dynamicparam { $script:events += 'd' }
}
$events = ''; 1,2 | Read-PhaseOrder; $events
$events = ''; Read-PhaseOrder; $events
$events = ''; @() | Read-PhaseOrder; $events
$phase = {param($p) clean { $script:events += 'c'; 'discard' } process { "process=$p/$_/$input" } end { "end=$p/$_/$input" } }
$events = ''; 4,5 | & $phase 'x'; $events
$events = ''; 'q' | . $phase 'y'; $events
function Read-Throws {process{ $script:events += "p$_"; if ($_ -eq 2) {throw 'broken'} } end { $script:events += 'e' } clean { $script:events += "c($_/$input)" } }
$events = ''; try { 1,2,3 | Read-Throws } catch { $script:events += "!$($_.Exception.Message)" }; $events
function Clean-Only { clean { $script:events += 'c' } }
function Clean-Begin { begin {} clean { $script:events += 'c' } }
function Clean-Record { process {} clean { $script:events += 'c' } }
$events = ''; Clean-Only; Clean-Begin; @() | Clean-Record; "empty=$events"
function Read-BeforeError {
    begin { 'started' }
    process { "seen=$_"; if ($_ -eq 2) { throw 'record failed' } }
    end { 'end' }
    clean { $script:events += 'cleaned' }
}
$events = ''
try { 1, 2, 3 | Read-BeforeError } catch { "caught=$($_.Exception.Message)" }
$events

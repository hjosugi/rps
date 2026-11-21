# rpsh-conformance: level=c2
try { throw 'boom' } catch { "caught: $_" } finally { 'finally' }
try { [int]'x' } catch [System.Management.Automation.RuntimeException] { 'runtime exception' }
try { 1 / 0 } catch { $_.Exception.GetType().FullName }
function F { trap { 'trapped'; continue }; throw 'x'; 'after' }
F

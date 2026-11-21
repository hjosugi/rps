# rpsh-conformance: level=c2
# docs/01_compat_contract.md, "Errors". Error ids, categories, exception
# types and messages are part of the answer. Messages come from upstream's
# resources and, for exceptions .NET raises, from the .NET 11.0.0 runtime.
function Show($record) {
    $record.FullyQualifiedErrorId
    "  $($record.CategoryInfo)"
    "  $($record.Exception.GetType().FullName)"
    "  $($record.Exception.Message)"
}
try { 1 / 0 } catch { Show $_ }
try { [int]::Parse('x') } catch { Show $_ }
try { Get-NoSuchThing } catch { Show $_ }
try { throw 'boom' } catch { Show $_ }
try { [int]'abc' } catch { Show $_ }
try { $null.Foo() } catch { Show $_ }
try { Get-Item /no/such/path -ErrorAction Stop } catch { Show $_ }
try { [System.IO.File]::ReadAllText('/no/such/file') } catch { Show $_ }
try { 'abc'.Substring(10) } catch { Show $_ }

# rpsh-conformance: level=c2
1..2 | ForEach-Object -Begin { 'begin' } -Process { $_ } -End { 'end' }

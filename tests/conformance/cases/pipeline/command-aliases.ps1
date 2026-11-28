# rpsh-conformance: level=c2
1, 2, 3 | ? { $_ -gt 1 } | % { $_ * 10 }
'ab', 'cde' | foreach -MemberName Length
% -Begin { 'begin' } -Process { } -End { 'end' }
$value = % -Process { } -End { 'assigned' }
$value
(% -Process { } -End { 'grouped' })
if (% -Process { } -End { $true }) { 'condition' }
foreach ($item in 2, 3) { $item % 2 }
$true ? 'yes' : 'no'

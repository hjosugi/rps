# rpsh-conformance: level=c2
(1..5 | Measure-Object -Sum).Sum
(1..5 | Measure-Object -Average).Average
(1..5 | Measure-Object -Minimum -Maximum | ForEach-Object { "$($_.Minimum) $($_.Maximum)" })
(1..5 | Select-Object -Last 2) -join ','
(1,1,2,2,3,1 | Get-Unique) -join ','
('a', 'b', 'c' | Measure-Object).Count

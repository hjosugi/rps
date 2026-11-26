# rpsh-conformance: level=c2
$ts = [timespan]::FromSeconds(90)
$ts.TotalSeconds
$ts.Minutes
$ts.Seconds
$ts.ToString()
$ts
New-TimeSpan -Seconds 90
(New-TimeSpan -Seconds 90).TotalSeconds
[timespan]::FromMilliseconds(1500).Milliseconds
[timespan]::FromHours(25).Days
[timespan]::FromHours(25).Hours
[timespan]::new(1, 2, 3, 4).ToString()
[timespan]::Zero.TotalSeconds
[timespan]::FromSeconds(1.2345).Ticks
[timespan]::FromTicks(123456789).ToString()
[timespan]::new(1, 2, 3, 4, 5).ToString()
[timespan]::Parse('00:01:30').TotalSeconds

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
(([datetime]'2024-01-31' + [timespan]::FromDays(1))).ToString('yyyy-MM-dd')
(([datetime]'2024-02-01' - [timespan]::FromDays(1))).ToString('yyyy-MM-dd')
([datetime]'2024-02-01' - [datetime]'2024-01-31').TotalDays
([timespan]::FromMinutes(1) + [timespan]::FromSeconds(30)).TotalSeconds
([timespan]::FromMinutes(2) - [timespan]::FromSeconds(30)).TotalSeconds
(-[timespan]::FromSeconds(90)).TotalSeconds
(([datetime]'2024-01-31 23:30:00' + [timespan]::FromMinutes(45))).ToString('yyyy-MM-dd HH:mm:ss')
[datetime]'2024-01-01' -lt [datetime]'2024-02-01'
[datetime]'2024-01-01' -eq [datetime]'2024-01-01'
[timespan]::FromSeconds(90) -gt [timespan]::FromSeconds(60)
[timespan]::FromSeconds(90) -le [timespan]::FromSeconds(90)
(New-TimeSpan -Start ([datetime]'2024-01-01') -End ([datetime]'2024-01-02')).TotalDays
(New-TimeSpan -Start '2024-01-01' -End '2024-01-01 12:00:00').TotalHours
([datetime]'2024-02-01', [datetime]'2024-01-01' | Sort-Object | ForEach-Object { $_.ToString('yyyy-MM-dd') }) -join ','
([timespan]::FromSeconds(90), [timespan]::FromSeconds(30) | Sort-Object | ForEach-Object TotalSeconds) -join ','

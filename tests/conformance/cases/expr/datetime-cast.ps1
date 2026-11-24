# rpsh-conformance: level=c2
$d = [datetime]'2024-03-04'
$d.Year
$d.Month
$d.Day
$d.ToString('yyyy-MM-dd')
$d.ToString('D')
([datetime]'2024-03-04').DayOfWeek
([datetime]'2024-03-04').DayOfYear
([datetime]'2023-12-31').DayOfYear
([datetime]'2024-02-29').DayOfYear

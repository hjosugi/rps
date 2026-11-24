# rpsh-conformance: level=c2
('a', 'b', 'c' | Join-String -Separator ',')
([pscustomobject]@{ Name = 'x' }, [pscustomobject]@{ Name = 'y' } | Join-String -Property Name -Separator '+')
'a', 'b' | Join-String -Separator ',' -DoubleQuote
'a', 'b' | Join-String -Separator ',' -SingleQuote
'1', '2' | Join-String -Separator ',' -OutputPrefix '[' -OutputSuffix ']'
'a', 'b' | Join-String -FormatString '<{0}>' -Separator ','

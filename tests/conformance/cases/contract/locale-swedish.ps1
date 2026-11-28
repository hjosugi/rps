# rpsh-conformance: level=c2 LANG=sv_SE.UTF-8
('Ö','z','Å','A','ä' | Sort-Object) -join '|'
('Ö','z','Å','A','ä' | Sort-Object -Culture en-US) -join '|'
'z' -gt 'ä'
'z'.CompareTo('ä') -lt 0

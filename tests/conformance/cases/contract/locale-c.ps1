# rpsh-conformance: level=c2 LANG=C
"$env:LANG|$env:LC_ALL"
('I','ı','i','İ' | Sort-Object -Unique -Stable) -join '|'
'İ' -like '[i]'
'ä'.CompareTo('z') -lt 0

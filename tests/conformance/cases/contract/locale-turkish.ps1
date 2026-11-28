# rpsh-conformance: level=c2 LANG=tr_TR.UTF-8
"$env:LANG|$env:LC_ALL"
'I' -eq 'i'
'I' -like 'ı'
'I'.CompareTo('i')
('I','ı','i','İ' | Sort-Object -Unique -Stable) -join '|'

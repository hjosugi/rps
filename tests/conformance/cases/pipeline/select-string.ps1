# rpsh-conformance: level=c2
'alpha', 'beta', 'gamma', 'delta' | Select-String -Pattern 'mm' | ForEach-Object { "$($_.LineNumber):$($_.Line)" }
'alpha', 'beta', 'gamma', 'delta' | Select-String -Pattern 'mm' -NotMatch | ForEach-Object { "$($_.LineNumber):$($_.Line)" }
'alpha', '[a]', 'beta' | Select-String -Pattern '[a]' -SimpleMatch | ForEach-Object { "$($_.LineNumber):$($_.Line)" }
'Alpha', 'alpha' | Select-String -Pattern 'alpha' -SimpleMatch | ForEach-Object Line
'Alpha', 'alpha' | Select-String -Pattern 'alpha' -SimpleMatch -CaseSensitive | ForEach-Object Line
'Alpha', 'alpha' | Select-String -Pattern 'alpha' -CaseSensitive | ForEach-Object Line
'alpha', 'beta' | Select-String -Pattern 'ph' -Quiet
'alpha', 'beta' | Select-String -Pattern 'zz' -Quiet
(Select-String -Pattern 'a' -Path ../cmdlets/fixtures/tree/a.txt | ForEach-Object Line) -join ','
(Select-String -Pattern 'a' -Path ../cmdlets/fixtures/tree/*.txt | ForEach-Object Line) -join ','
'alpha', 'beta', 'gamma' | Select-String -Pattern 'ph', 'et' | ForEach-Object Line
'alpha', 'beta', 'gamma' | Select-String -Pattern 'ph', 'et' -SimpleMatch | ForEach-Object Line
'alpha', 'beta', 'gamma' | Select-String -NotMatch -Pattern 'ph', 'et' | ForEach-Object Line
'alpha', 'beta', 'gamma' | Select-String -Pattern 'ph', 'et' -NotMatch | ForEach-Object Line

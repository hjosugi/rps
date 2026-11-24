# rpsh-conformance: level=c2
'alpha', 'beta', 'gamma', 'delta' | Select-String -Pattern 'mm' | ForEach-Object { "$($_.LineNumber):$($_.Line)" }
'alpha', 'beta', 'gamma', 'delta' | Select-String -Pattern 'mm' -NotMatch | ForEach-Object { "$($_.LineNumber):$($_.Line)" }
'alpha', '[a]', 'beta' | Select-String -Pattern '[a]' -SimpleMatch | ForEach-Object { "$($_.LineNumber):$($_.Line)" }
'Alpha', 'alpha' | Select-String -Pattern 'alpha' -SimpleMatch | ForEach-Object Line
'Alpha', 'alpha' | Select-String -Pattern 'alpha' -SimpleMatch -CaseSensitive | ForEach-Object Line
'Alpha', 'alpha' | Select-String -Pattern 'alpha' -CaseSensitive | ForEach-Object Line

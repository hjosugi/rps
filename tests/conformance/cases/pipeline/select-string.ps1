# rpsh-conformance: level=c2
'alpha', 'beta', 'gamma', 'delta' | Select-String -Pattern 'mm' | ForEach-Object { "$($_.LineNumber):$($_.Line)" }
'alpha', 'beta', 'gamma', 'delta' | Select-String -Pattern 'mm' -NotMatch | ForEach-Object { "$($_.LineNumber):$($_.Line)" }
'alpha', '[a]', 'beta' | Select-String -Pattern '[a]' -SimpleMatch | ForEach-Object { "$($_.LineNumber):$($_.Line)" }

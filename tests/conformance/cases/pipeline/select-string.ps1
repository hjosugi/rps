# rpsh-conformance: level=c2
'alpha', 'beta', 'gamma', 'delta' | Select-String -Pattern 'mm' | ForEach-Object { "$($_.LineNumber):$($_.Line)" }

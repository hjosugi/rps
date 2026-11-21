# rpsh-conformance: level=c2
# Out-String formats each pipeline on its own.
[pscustomobject]@{ Name = 'a'; Value = 1 }, [pscustomobject]@{ Name = 'bb'; Value = 22 } | Out-String
# Up to four properties make a table, more make a list.
[pscustomobject]@{ A = 1; B = 2; C = 3; D = 4; E = 5 } | Out-String
[ordered]@{ key = 'value'; other = 2 } | Out-String
3.5, 'text', $true | Out-String

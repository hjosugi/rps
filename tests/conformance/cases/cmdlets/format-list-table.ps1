# rpsh-conformance: level=c2
[pscustomobject]@{ A = 1; B = 2 } | Format-List | Out-String
[pscustomobject]@{ A = 1; B = 2; C = 3; D = 4; E = 5 } | Format-Table | Out-String

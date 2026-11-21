# rpsh-conformance: level=c2
# The first object decides the table layout for the rest of the output
# stream: later objects of another shape are squeezed into its columns.
[pscustomobject]@{ Name = 'a'; Value = 1 }, [pscustomobject]@{ Name = 'bb'; Value = 22 }
[pscustomobject]@{ A = 1; B = 2; C = 3; D = 4; E = 5 }
[ordered]@{ key = 'value'; other = 2 }

# rpsh-conformance: level=c2
# Type tests and conversions with -is / -isnot / -as and type literals.
1 -is [int]
'a' -is [int]
1 -isnot [string]
1 -as [int]
'x' -as [int]

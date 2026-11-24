# rpsh-conformance: level=c2
([pscustomobject]@{ A = 1; B = 2 } | Get-Member -MemberType NoteProperty | ForEach-Object Name | Sort-Object) -join ','
([pscustomobject]@{ A = 1; B = 2 } | Get-Member -Name A).Name
([pscustomobject]@{ A = 1; B = 2 } | Get-Member -Name 'A*' | ForEach-Object Name) -join ','

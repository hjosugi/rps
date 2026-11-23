# rpsh-conformance: level=c2
$people = [pscustomobject]@{ Name = 'a'; Age = 20 }, [pscustomobject]@{ Name = 'b'; Age = 30 }
($people | Where-Object Age -gt 25 | ForEach-Object Name) -join ','
($people | Where-Object Name -eq 'a' | ForEach-Object Age) -join ','

# rpsh-conformance: level=c2
$people = [pscustomobject]@{ Name = 'b'; Age = 30 }, [pscustomobject]@{ Name = 'a'; Age = 20 }
($people | Sort-Object -Property Age | ForEach-Object Name) -join ','
($people | Select-Object -Property Name | ForEach-Object { $_.Name }) -join ','
($people | Select-Object -ExpandProperty Name) -join ','

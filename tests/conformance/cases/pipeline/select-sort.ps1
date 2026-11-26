# rpsh-conformance: level=c2
$people = [pscustomobject]@{ Name = 'b'; Age = 30 }, [pscustomobject]@{ Name = 'a'; Age = 20 }
($people | Sort-Object -Property Age | ForEach-Object Name) -join ','
($people | Select-Object -Property Name | ForEach-Object { $_.Name }) -join ','
($people | Select-Object -ExpandProperty Name) -join ','
$people = [pscustomobject]@{ Name = 'a'; Age = 30 }, [pscustomobject]@{ Name = 'b'; Age = 20 }, [pscustomobject]@{ Name = 'c'; Age = 30 }
($people | Select-Object -Property Age -Unique | ForEach-Object Age) -join ','
([pscustomobject]@{ A = 1; B = 2 } | Select-Object * | ForEach-Object { "$($_.A)$($_.B)" })
([pscustomobject]@{A=1;AB=2;B=3} | Select-Object -Property 'A*' | ForEach-Object { $_.psobject.Properties.Name }) -join ','
([pscustomobject]@{A=1;B=2} | Select-Object -Property * | ForEach-Object { $_.psobject.Properties.Name }) -join ','

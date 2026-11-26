# rpsh-conformance: level=c2
$people = [pscustomobject]@{ Name = 'a'; Age = 20 }, [pscustomobject]@{ Name = 'b'; Age = 20 }, [pscustomobject]@{ Name = 'c'; Age = 30 }
($people | Group-Object -Property Age | ForEach-Object { "$($_.Name):$($_.Count)" }) -join ','
($people | Measure-Object -Property Age -Sum).Sum
($people | Measure-Object -Property Age -Average).Average
('a', 'A' | Group-Object -CaseSensitive | ForEach-Object { "$($_.Name):$($_.Count)" }) -join ','
('a', 'A' | Group-Object | ForEach-Object { "$($_.Name):$($_.Count)" }) -join ','
('a', 'b', 'a' | Group-Object -NoElement | ForEach-Object { "$($_.Name):$($_.Count)" }) -join ','
$h = 'a', 'b', 'a' | Group-Object -AsHashTable
$h['a'].Count
$h['b'][0]
$multi = @([pscustomobject]@{A=1;B='x'}, [pscustomobject]@{A=1;B='x'}, [pscustomobject]@{A=2;B='y'})
($multi | Group-Object -Property A, B | ForEach-Object { "$($_.Name)|$($_.Count)|$($_.Values.Count)" }) -join ';'
($multi | Group-Object -Property A, B -NoElement | ForEach-Object { "$($_.Name)|$($_.Count)" }) -join ';'

# rpsh-conformance: level=c2
$people = [pscustomobject]@{ Name = 'a'; Age = 20 }, [pscustomobject]@{ Name = 'b'; Age = 30 }
($people | Where-Object Age -gt 25 | ForEach-Object Name) -join ','
($people | Where-Object Name -eq 'a' | ForEach-Object Age) -join ','
$truth = @([pscustomobject]@{Name=''}, [pscustomobject]@{Name='x'})
($truth | Where-Object Name | ForEach-Object Name) -join ','
($truth | Where-Object -Property Name | ForEach-Object Name) -join ','

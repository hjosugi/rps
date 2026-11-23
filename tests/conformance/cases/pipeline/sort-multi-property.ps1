# rpsh-conformance: level=c2
$data = @([pscustomobject]@{A=2;B='b'}, [pscustomobject]@{A=1;B='z'}, [pscustomobject]@{A=2;B='a'})
($data | Sort-Object -Property @('A', 'B') | ForEach-Object { "$($_.A)$($_.B)" }) -join ','
($data | Sort-Object -Property @('A', 'B') -Descending | ForEach-Object { "$($_.A)$($_.B)" }) -join ','

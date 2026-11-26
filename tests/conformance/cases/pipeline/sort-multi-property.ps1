# rpsh-conformance: level=c2
$data = @([pscustomobject]@{A=2;B='b'}, [pscustomobject]@{A=1;B='z'}, [pscustomobject]@{A=2;B='a'})
($data | Sort-Object -Property @('A', 'B') | ForEach-Object { "$($_.A)$($_.B)" }) -join ','
($data | Sort-Object -Property @('A', 'B') -Descending | ForEach-Object { "$($_.A)$($_.B)" }) -join ','
($data | Sort-Object -Property A, B | ForEach-Object { "$($_.A)$($_.B)" }) -join ','
($data | Sort-Object -Property A, B -Descending | ForEach-Object { "$($_.A)$($_.B)" }) -join ','
Write-Output a, b
$wild = @([pscustomobject]@{A1=2;A2=1}, [pscustomobject]@{A1=1;A2=2})
($wild | Sort-Object -Property 'A*' | ForEach-Object { "$($_.A1)$($_.A2)" }) -join ','
($wild | Sort-Object -Property * | ForEach-Object { "$($_.A1)$($_.A2)" }) -join ','

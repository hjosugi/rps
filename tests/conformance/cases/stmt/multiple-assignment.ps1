# rpsh-conformance: level=c2
$a, $b = 1, 2
"$a|$b|$($b.GetType().Name)"
$a, $b = $b, $a
"$a|$b"
$head, $tail = 1..4
"$head|$($tail -join ',')|$($tail.GetType().Name)"
$first, $second, $third = 'scalar'
"$first|$($null -eq $second)|$($null -eq $third)"
$first, $second = @()
"$($null -eq $first)|$($null -eq $second)"
$first, $second = $null, 2
"$($null -eq $first)|$second"
$first, $second = (1, 2), (3, 4)
"$($first -join ',')|$($second -join ',')"
($first, $second), $third = (1, 2, 3), 4
"$first|$($second -join ',')|$third"
$first, $second, $first = 1, 2, 3, 4
"$($first -join ',')|$second"
$a, $b = Write-Output 8 9 10
"$a|$($b -join ',')"
$a, $b = 1..4 | Where-Object { $_ -gt 2 }
"$a|$b|$($b.GetType().Name)"
$a, $b = 'ignored' | Out-Null
"$($null -eq $a)|$($null -eq $b)"
$record = @{ Name = 'old'; Kind = 'old' }
$record.Name, $record['Kind'] = 'new', 'value'
"$($record.Name)|$($record.Kind)"
function Set-Pair { $global:left, $global:right = Write-Output 10 20 }
Set-Pair
"$left|$right"

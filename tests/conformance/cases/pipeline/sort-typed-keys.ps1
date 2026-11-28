# rpsh-conformance: level=c2
$inputNames = 'z', 'å', 'ä', 'a', 'A', 'ö', 'Z', 'é', ('e' + [char]0x301)
($inputNames | Sort-Object -Culture sv-SE -Stable) -join ','
($inputNames | Sort-Object -Culture en-US -Stable) -join ','
($inputNames | Sort-Object -Culture sv-SE -CaseSensitive -Unique) -join ','
($inputNames | Sort-Object -Culture 1053 -Unique -Descending -Stable) -join ','
('I', 'i', 'İ', 'ı' | Sort-Object -Culture tr-TR -Unique -Stable) -join ','
('I', 'i', 'İ', 'ı' | Sort-Object -Culture '0x41f' -CaseSensitive) -join ','
(-2, 10, 2, 100, -11, 0 | Sort-Object) -join ','
(18446744073709551615ul, 2, 18446744073709551614ul | Sort-Object) -join ','
(9.01d, 9d, 0.001d, -100d, -10d, 0d | Sort-Object) -join ','
([timespan]::FromSeconds(-1),[timespan]::FromSeconds(-10),[timespan]::FromSeconds(2) | Sort-Object | ForEach-Object TotalSeconds) -join ','
$rows = @([pscustomobject]@{A='a';B='bc';Id=1},[pscustomobject]@{A='ab';B='c';Id=2},[pscustomobject]@{A='a';B='bc';Id=3},[pscustomobject]@{A='a';B='b';Id=4})
($rows | Sort-Object A, B -Stable | ForEach-Object Id) -join ','
($rows | Sort-Object -Property A, B -Unique | ForEach-Object Id) -join ','
($rows | Sort-Object -Property A, B -Stable -Descending | ForEach-Object Id) -join ','
($rows | Sort-Object @{Expression='A';Descending=$true}, @{e='B';Ascending=$true} | ForEach-Object Id) -join ','
$items = @([pscustomobject]@{A=$null;Id='null'},[pscustomobject]@{B=1;Id='missing'},[pscustomobject]@{A=-1;Id='negative'},[pscustomobject]@{A=0;Id='zero'})
($items | Sort-Object A | ForEach-Object Id) -join ','
($items | Sort-Object A -Descending | ForEach-Object Id) -join ','
$by = { $_ % 3 }
(1..20 | Sort-Object $by -Stable) -join ','
(1..20 | Sort-Object { $_ % 3 } -Stable -Descending -Unique) -join ','
(1..20 | Sort-Object @{ e=$by; Ascending=$false } -Top 5) -join ','
(1..20 | Sort-Object $by -Bottom 4) -join ','
(1..20 | Sort-Object $by) -join ','
(10,1,10,2,1 | Sort-Object -Top 2 -Unique) -join ','
(10,1,10,2,1 | Sort-Object -Bottom 2 -Unique) -join ','
(Sort-Object -InputObject @(3,1,2)) -join ','
$bad = [string][char]0xd800
('a', $bad, 'b' | Sort-Object | ForEach-Object { [int]$_[0] }) -join ','
try { 1 | Sort-Object -Bottom 0 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 1 | Sort-Object -Top 1 -Stable } catch { $_.FullyQualifiedErrorId }
(2,1,3 | Sort-Object -Descending:$false) -join ','
($rows | Sort-Object -Desc A | ForEach-Object Id) -join ','
($rows | Sort-Object -prop Id -Desc:$true | ForEach-Object Id) -join ','
try { 1 | Sort-Object -c } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 1 | Sort-Object -Unknown } catch { $_.FullyQualifiedErrorId }
try { 1 | Sort-Object -Culture 'a b' } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
$duplicates = 0.5,1.1d,-99,7l,7l,-99,0,1.1d,9007199254740993ul,9.991d,-99,5,9.991d,4u,9.991d
($duplicates | Sort-Object -Bottom 3 -Unique) -join ','
('İ','カ','I','𐐀','a','i','𐐀','é','é','𐐀','I','か','Ä','æ','i' | Sort-Object -Culture en-US -Top 4 -Unique) -join ','

# rpsh-conformance: level=c2
foreach ($flags in @('None', 'IgnoreCase', 'CultureInvariant', 'IgnoreCase, CultureInvariant', 'Compiled, IgnoreCase')) {
    $pattern = [WildcardPattern]::Get('a[BC]?', $flags)
    "$flags=$($pattern.IsMatch('ABx')),$($pattern.IsMatch('abx')),$($pattern.IsMatch('aBx'))"
    [WildcardPattern]::new('i?', $flags).IsMatch('İx')
    [WildcardPattern]::new('[i]', $flags).IsMatch('İ')
}
$decomposed = 'e' + [char]0x301
$decomposed -clike 'é'
($decomposed + 'x') -clike 'é*'
($decomposed + 'x') -clike 'e*'
$decomposed -clike '*é'
('a' + [char]0 + 'b') -like 'a?b'
('a' + [char]10 + 'b') -like 'a*b'
'😀' -like '?'
'😀' -like '??'
([char]0xd800) -clike ([char]0xd801)
([string][char]0xd800) -clike '?'
'ABC-' -like 'a*[b-c][`-]'
'k' -like '[K]'
'_' -like '[A-z]'
']' -like '[]]'
'`' -like '``'
'' -like '`'
'*' -clike '`*'
'foo' -notlike '?o*o?'
('one', 'TWO', 'too' -like 't??') -join '|'
(11, 2 -like '?')[0].GetType().FullName
$p = [WildcardPattern]::new('A', 'IgnoreCase')
'a' -clike $p
switch -CaseSensitive -Wildcard ('a') { $p { 'matched' } default { 'default' } }
[WildcardPattern]::new('*').IsMatch($null)
[WildcardPattern]::new('*').IsMatch([NullString]::Value)
try { [WildcardPattern]::new('[z-a]').IsMatch([NullString]::Value) } catch { $_.FullyQualifiedErrorId }
try { 'a' -like '[a' } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { switch -Wildcard ('a') { '[a' { 'bad' } } } catch { $_.FullyQualifiedErrorId }
(Get-Command -Name 'Get-Ali[a]s').Name
(Get-Alias -Name 'g[c]').Name
try { Get-Command -Name '[' -ErrorAction Stop } catch { $_.FullyQualifiedErrorId }
try { Get-Alias -Name '[' -ErrorAction Stop } catch { $_.FullyQualifiedErrorId }

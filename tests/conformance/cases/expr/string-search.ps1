# rpsh-conformance: level=c2
$s='😀' + 'é' + [char]0 + 'aÉ'
$d='e' + [char]0x301
$s.Length
[int]$s[0]
[int]$s[1]
[int]$s.Substring(1,1)[0]
$s.IndexOf('é')
$s.IndexOf($d)
$s.IndexOf($d,[StringComparison]::Ordinal)
$s.IndexOf($d,3,[StringComparison]::CurrentCultureIgnoreCase)
$s.LastIndexOf($d,[StringComparison]::CurrentCultureIgnoreCase)
$s.Contains($d)
$s.Contains($d,'InvariantCulture')
$s.StartsWith('😀','Ordinal')
$s.EndsWith($d,[StringComparison]::CurrentCultureIgnoreCase)
'ı'.StartsWith('I',$true,[CultureInfo]::new('tr-TR'))
$s.IndexOf([char]0)
$s.IndexOf([string][char]0)
$s.LastIndexOf([char]0)
$s.LastIndexOf([string][char]0)
$s.IndexOf([char]0xde00)
$s.IndexOf([char]0xd83d)
$s.IndexOf($null,[StringComparison]::Ordinal)
$s.IndexOf('',2,1)
$s.LastIndexOf('',2,1)
$s.LastIndexOf('',0,0)
$s.LastIndexOf('',6,0)
''.LastIndexOf('')
''.LastIndexOf([char]97,42,0)
'abc'.IndexOf(97)
'abc'.Contains(98)
('a' + [char]13 + [char]10 + 'b').IndexOf([string][char]10)
'😀'.IndexOf([string][char]0xd83d,[StringComparison]::Ordinal)
('I' + 'ı').IndexOf('i',[StringComparison]::InvariantCultureIgnoreCase)
'abc'.IndexOf('c',1,2,[StringComparison]::Ordinal)
'abc'.LastIndexOf('a',2,3,[StringComparison]::Ordinal)
'ABC'.Contains('a','OrdinalIgnoreCase')
([System.StringComparison]'ordinalignorecase').ToString()
try { 'abc'.IndexOf('a',-1) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 'abc'.LastIndexOf('a',1,3) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 'abc'.IndexOf('b','Ordinal') } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 'abc'.Contains([NullString]::Value) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 'abc'.Substring(-1) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 'abc'.Substring(1,3) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
''.LastIndexOf([char]97,42,0,[StringComparison]::CurrentCulture)
try { 'abc'.IndexOf([char]97,-1,0,[StringComparison]::OrdinalIgnoreCase) } catch { $_.Exception.Message }
try { 'abc'.LastIndexOf([char]97,0,2,[StringComparison]::OrdinalIgnoreCase) } catch { $_.Exception.Message }

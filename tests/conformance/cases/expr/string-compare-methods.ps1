# rpsh-conformance: level=c2 LANG=tr_TR.UTF-8
$d='e' + [char]0x301
foreach ($m in 'CurrentCulture','CurrentCultureIgnoreCase','InvariantCulture','InvariantCultureIgnoreCase','Ordinal','OrdinalIgnoreCase') {
    $mode=[StringComparison]$m
    [string]::Compare('I','ı',$mode)
    [string]::Compare('é',$d,$mode)
    [string]::Compare('😀','𐐀',$mode)
    [string]::Compare('a😀z',1,'b𐐀!',1,1,$mode)
    [string]::Equals('I','i',$mode)
    'é'.Equals($d,$mode)
}
[string]::Compare('I','ı',$true,[CultureInfo]::InvariantCulture)
[string]::Compare('I','ı',$true,[CultureInfo]::CurrentCulture)
[string]::Compare('😀',0,'𐐀',0,1)
[string]::Compare('ab',0,'abcd',0,20)
[string]::Compare('a','A','Ordinal')
[string]::CompareOrdinal('','x')
[string]::CompareOrdinal('x','xz')
[string]::CompareOrdinal('ab','abZ')
[string]::CompareOrdinal('abc',1,'zbc',1,100)
[string]::CompareOrdinal($null,'')
[string]::CompareOrdinal([NullString]::Value,'')
[string]::CompareOrdinal([NullString]::Value,-5,'',2,-3)
[string]::Equals('', $null)
[string]::Equals($null,'')
''.Equals($null)
''.Equals($null,'Ordinal')
[string]::Equals('1',1)
[string]::Equals(1,'1')
'1'.Equals(1)
'1'.Equals(1,'Ordinal')
try { [string]::Compare('abc',0,'d',0,-1) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { [string]::Compare('abc',-1,'d',0,2,[StringComparison]::Ordinal) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { [string]::Compare('a','b',1) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { [string]::Equals('a','b','unknown') } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }

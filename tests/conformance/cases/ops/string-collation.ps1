# rpsh-conformance: level=c2
$decomposed = 'e' + [char]0x301
'é' -ceq $decomposed
'é' -eq 'É'
'é' -ceq 'É'
'ﬀ' -eq 'ff'
'ﬀ' -ceq 'ff'
'Ａ' -eq 'ａ'
'Ａ' -eq 'a'
'か' -eq 'カ'
'_' -lt '1'
'ä' -lt 'z'
('a' + [char]0 + 'b') -ceq 'ab'
([string][char]0xd800) -ceq ([string][char]0xd801)
('a' + [char]0xd800) -ceq ('a' + [char]0xd801)
([char]0xd83d + [char]0xde00) -ceq '😀'
'😀' -ceq '😀'
'é'.CompareTo($decomposed)
'a'.CompareTo('A')
'A'.CompareTo('a')
'z'.CompareTo('ä')
''.CompareTo($null)
try { 'a'.CompareTo(1) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }

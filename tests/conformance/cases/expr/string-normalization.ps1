# rpsh-conformance: level=c2
$d = 'A' + [char]0x30a + [char]0x301
$d.IsNormalized()
$d.IsNormalized('FormD')
$d.Normalize()
$d.Normalize([Text.NormalizationForm]::FormD).Length
'Ǻ'.Normalize('FormD').Length
'ﬀＡ'.Normalize('FormKC')
'ﬀＡ'.Normalize(6)
'가'.Normalize('FormD').Length
$long = ('ﬁ' * 100).Normalize('FormKD')
$long.Length
''.Normalize().Length
''.IsNormalized(5)
('😀' + [char]0).Normalize().Length
([string][char]0xffff).IsNormalized()
[int][Text.NormalizationForm]::FormKD
([System.Text.NormalizationForm]'formkc').ToString()
([Text.NormalizationForm]::FormC).GetType().FullName
foreach ($value in @(([string][char]0xd800), ([string][char]0xdc00), ([string][char]0xfffe))) {
  try { $value.Normalize() } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
  try { $value.IsNormalized('FormC') } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
}
try { 'a'.Normalize($null) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 'a'.Normalize(7) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 'a'.Normalize('bogus') } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
'é'.Normalize(' FormD ').Length
'é'.Normalize('FormC,FormC').Length
try { 'a'.Normalize('0x2') } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 'a'.Normalize('FormK') } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { [Text.NormalizationForm]7 } catch { $_.FullyQualifiedErrorId }

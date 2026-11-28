# rpsh-conformance: level=c2
$s='iIİıſßﬀẞΣσςᾀ𐐨'
$s.ToUpper()
$s.ToLower()
$s.ToUpperInvariant()
$s.ToLowerInvariant()
$s.ToLower($null)
$s.ToUpper('')
foreach ($locale in 'en-US','tr-TR','az-Latn-AZ','') {
    $culture = [cultureinfo]$locale
    $culture.GetType().FullName
    $culture.ToString()
    $s.ToUpper($culture)
    $s.ToLower($culture)
}
$s.ToLower([Globalization.CultureInfo]::GetCultureInfo('tr-TR'))
$s.ToUpper([Globalization.CultureInfo]::new('tr-TR'))
$s.ToUpper([CultureInfo]::InvariantCulture)
$s.ToLower(1055)
([string][char]0xd800).ToUpperInvariant().Length
[int]('x' + [char]0xd800).ToUpper()[1]
try { 'a'.ToLower('invalid!') } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { 'a'.ToUpperInvariant('') } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }

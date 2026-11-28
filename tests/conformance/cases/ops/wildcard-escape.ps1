# rpsh-conformance: level=c2
foreach ($sample in @('', 'normal', '*?[]', 'a`', 'a`z', '`*', '``?', '漢😀[字]', 'a]b')) {
    $quoted = [WildcardPattern]::Escape($sample)
    $plain = [WildcardPattern]::Unescape($sample)
    $roundtrip = [WildcardPattern]::Unescape($quoted)
    "$sample|$quoted|$plain|$roundtrip"
    [WildcardPattern]::ContainsWildcardCharacters($sample)
    [WildcardPattern]::ContainsWildcardCharacters($quoted)
}
"null=$([WildcardPattern]::Escape($null))|$([WildcardPattern]::Unescape($null))"
[WildcardPattern]::ContainsWildcardCharacters($null)
try { [System.Management.Automation.WildcardPattern]::Escape([NullString]::Value) } catch { $_.FullyQualifiedErrorId }
try { [WildcardPattern]::Unescape([NullString]::Value) } catch { $_.FullyQualifiedErrorId }
[WildcardPattern]::ContainsWildcardCharacters([NullString]::Value)

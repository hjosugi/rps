# rpsh-conformance: level=c2
$flags = [System.Management.Automation.WildcardOptions]::IgnoreCase -bor [System.Management.Automation.WildcardOptions]::CultureInvariant
$flags
$flags.GetType().FullName
$flags.value__
1 -bor $flags
foreach ($source in @('', 'a*?', '%_x', '`[`]', '[[]', 'file[0-9].txt', '[Z-_]', 'a`', '`', '`*', '[]]', '[-]', '[^-_]', '[', '[z-a]')) {
    $pattern = [WildcardPattern]::Get($source, $flags)
    try { "$source=>$($pattern.ToWql())" } catch { "$source=>$($_.FullyQualifiedErrorId)" }
}
$n = [System.Management.Automation.WildcardPattern]::new('x')
$n.GetType().FullName
$n.ToString()
[WildcardPattern]::new('test', 'Compiled, IgnoreCase').ToWql()
try { [WildcardPattern]::new('x', 8) } catch { $_.FullyQualifiedErrorId }
try { [WildcardPattern]::new('x', $null) } catch { $_.FullyQualifiedErrorId }
try { [WildcardPattern]::new([NullString]::Value) } catch { $_.FullyQualifiedErrorId }

# rpsh-conformance: level=c2 LANG=fr_FR.UTF-8
$root = Join-Path $PSScriptRoot 'fixtures/help-modules'
$env:PSModulePath = "$root/current:$root/archive:$env:PSModulePath"
$apple = Get-Help about_rpsh_Apple -Detailed
"$PSUICulture|$($apple.Name)|$($apple.Synopsis)|$($apple.Contains('Deuxième édition'))"
$pear = Get-Help about_rpsh_Pear
"$($pear.Name)|$($pear.Synopsis)|$($pear.Contains('A second tree'))"

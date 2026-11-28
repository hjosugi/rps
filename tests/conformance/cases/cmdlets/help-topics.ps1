# rpsh-conformance: level=c2
$root = Join-Path $PSScriptRoot 'fixtures/help-modules'
$env:PSModulePath = "$root/current:$root/archive:$env:PSModulePath"
$default = Get-Help
"$($default.Name)|$($default.Category)|$($default.GetType().FullName)|$($default.Synopsis)|$($default.Length)"
$default.Contains('Get-Help')
$apple = Get-Help about_rpsh_Apple
"$($apple.Name)|$($apple.PSObject.TypeNames -join ',')|$($apple[0])|$($apple.Substring(0,5))"
Get-Help about_rpsh_Apple -Category HelpFile
$both = Get-Help 'about_rpsh_*' | Sort-Object Name
foreach ($topic in $both) {
    "$($topic.Name)|$($topic.Category)|$($topic.Synopsis)|$($topic.PSTypeNames -join ',')"
}
$both | Out-String -Width 100
(Get-Help about_rpsh_Apple -Full).PSTypeNames -join ','
$pear = Get-Help '*rpsh_Pear' -Examples
"$($pear.Name)|$($pear.PSTypeNames -join ',')|$($pear.Contains('A second tree'))"
(Get-Help espalier -Category HelpFile | Sort-Object Name).Name -join ','
$missing = Get-Help 'about_rpsh_missing*'
@($missing).Count
try { Get-Help about_rpsh_missing -ErrorAction Stop } catch { $_.FullyQualifiedErrorId }
try { Get-Help about_rpsh_Pear -Parameter '*' -EA Stop } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Get-Help about_rpsh_Pear -Category HelpFile -Role '*' } catch { $_.FullyQualifiedErrorId }
try { Get-Help about_rpsh_Pear -Online } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
(Get-Help "$root/archive/FruitDocs/1.0.0/about_rpsh_Apple").Synopsis

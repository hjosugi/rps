# rpsh-conformance: level=c2 LANG=C
foreach ($name in 'ErrorActionPreference','WarningPreference','ProgressPreference','VerbosePreference','DebugPreference','InformationPreference') {
    $p=Get-Variable -Name $name -ValueOnly
    "$name|$p|$([int]$p)|$($p.GetType().FullName)"
}
[System.Management.Automation.ActionPreference]::Continue.ToString()
$ErrorActionPreference='sil'
"$ErrorActionPreference|$([int]$ErrorActionPreference)"
$ErrorActionPreference=[System.Management.Automation.ActionPreference]::Stop
$ErrorActionPreference.GetType().FullName
foreach($v in @($null,'','bad',7,'Suspend',$true)) {
    try { $ErrorActionPreference=$v } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
}
$ErrorActionPreference='Continue'
foreach ($v in @(0,1,3,'false',$null)) {
    try { $PSNativeCommandUseErrorActionPreference=$v; "$PSNativeCommandUseErrorActionPreference|$($PSNativeCommandUseErrorActionPreference.GetType().FullName)" }
    catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
}

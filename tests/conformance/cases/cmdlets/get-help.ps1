# rpsh-conformance: level=c2
foreach ($name in 'Write-Output','Sort-Object','Switch-Process','Get-Help') {
    $h = Get-Help $name
    $short = & $name -?
    "$($h.Name)|$($h.Category)|$($h.ModuleName)|$($h.details.Verb)|$($h.details.Noun)"
    $h.Synopsis -ceq $short.Synopsis
    $h.PSTypeNames -join ','
    "$($h.syntax.syntaxItem.Count)|$($h.parameters.parameter.Count)|$($h.relatedLinks.navigationLink[0].uri)"
    $h.parameters.parameter | ForEach-Object { "$($_.name):$($_.position):$($_.pipelineInput):$($_.parameterSetName)" }
    [System.Management.Automation.ProxyCommand]::GetHelpComments($h)
}
$w = Get-Help echo -Full
$w.psobject.TypeNames -join ','
$p = Get-Help Write-Output -Parameter 'Input*'
"$($p.name)|$($p.required)|$($p.type.name)|$($p.parameterValue.required)|$($p.parameterValue.variableLength)"
(Get-Help Sort-Object -Parameter 'T*','*op').name -join ','
(Get-Help Write-Output -Category Cmdlet -Role '*').Name
$h = Get-Help Switch-Process
$h.PSSnapIn.ApplicationBase -eq $PSHOME
$h.PSSnapIn.Name
$g = Get-Help Get-Help
$g.Syntax.syntaxItem[0].parameter[2].parameterValueGroup.parameterValue -join ','
$h = Write-Output ignored -? -Unknown
$h.Name
Write-Output -- -?
Write-Output '-?'
Get-Help Write-Output | Out-String -Width 45
Get-Help Write-Output -Parameter NoEnumerate | Out-String -Width 60
[pscustomobject]@{Name='Write-Output'} | Get-Help | ForEach-Object { $_.Name }
try { Get-Help Write-Output -Parameter notAParameter -EA Stop } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Get-Help Sort-Object -Full -Detailed } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Get-Help Write-Output -Category invalid } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
$p = (Get-Help Write-Output).parameters.parameter[0].parameterValue
"$($p.ToString())|$($p.GetType().FullName)|$($p.PSObject.TypeNames -join ',')"
"$($w.ToString())|$($w.details.ToString())"
try { 42 | Get-Help -ErrorAction Stop } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Get-Help -Name '' } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
Set-Alias zHelpOut Write-Output
Set-Alias aHelpOut Write-Output
Set-Alias chainedHelpOut zHelpOut
(Get-Help chainedHelpOut).aliases
Set-Alias zHelpOut Write-Output
(Get-Help Write-Output).aliases
(chainedHelpOut -?).Name

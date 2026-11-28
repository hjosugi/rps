# rpsh-conformance: level=c2
$m = [System.Management.Automation.CommandMetadata]::new((Get-Command Write-Output))
[System.Management.Automation.ProxyCommand]::Create($m)
foreach ($n in 'Sort-Object','Switch-Process') {
    $meta = [System.Management.Automation.CommandMetadata]::new((Get-Command -Name $n))
    [System.Management.Automation.ProxyCommand]::GetCmdletBindingAttribute($meta)
    [System.Management.Automation.ProxyCommand]::GetParamBlock($meta)
    [System.Management.Automation.ProxyCommand]::GetDynamicParam($meta)
}
$m.Name = 'renamed'
$m.DefaultParameterSetName = "user's set"
$m.HelpUri = "a'url"
$m.SupportsShouldProcess = $true
$m.ConfirmImpact = 'High'
$m.PositionalBinding = $false
$m.Parameters.NoEnumerate.Aliases.Add("one'alias")
$m.Parameters.NoEnumerate.Aliases.Add('two')
$m.Parameters.InputObject.Attributes.Clear()
[System.Management.Automation.ProxyCommand]::Create($m,'custom <# nested #> help',$false)
$fromType = [System.Management.Automation.CommandMetadata]::new([Microsoft.PowerShell.Commands.WriteOutputCommand])
[System.Management.Automation.ProxyCommand]::GetBegin($fromType)
[System.Management.Automation.ProxyCommand]::GetProcess($fromType)
[System.Management.Automation.ProxyCommand]::GetEnd($fromType)
[System.Management.Automation.ProxyCommand]::GetClean($fromType)
try { [System.Management.Automation.ProxyCommand]::GetBegin($null) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { [System.Management.Automation.ProxyCommand]::Create() } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
$copy = [System.Management.Automation.CommandMetadata]::new($m)
[System.Management.Automation.ProxyCommand]::GetCmdletBindingAttribute($copy)
[object]::ReferenceEquals($m.Parameters.NoEnumerate.Attributes[0],$copy.Parameters.NoEnumerate.Attributes[0])
$empty = [System.Management.Automation.CommandMetadata]::new([int])
[System.Management.Automation.ProxyCommand]::GetCmdletBindingAttribute($empty)
$empty.Parameters.Count
try { [System.Management.Automation.ProxyCommand]::Create($empty) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { [System.Management.Automation.ProxyCommand]::GetEnd($false) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { [System.Management.Automation.ProxyCommand]::GetParamBlock(@(1,2)) } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }

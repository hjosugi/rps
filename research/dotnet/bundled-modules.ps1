# The modules the reference pwsh ships in $PSHOME/Modules: which are binary (load a
# DLL) and which assemblies they bring (docs/06_dotnet_bridge.md).
$ErrorActionPreference = 'Stop'
foreach ($directory in Get-ChildItem (Join-Path $PSHOME 'Modules') -Directory | Sort-Object Name) {
    $manifest = Get-ChildItem $directory.FullName -Recurse -Filter "$($directory.Name).psd1" | Select-Object -First 1
    $data = if ($manifest) { Import-PowerShellDataFile $manifest.FullName } else { @{} }
    $root = $data.RootModule
    if (-not $root) { $root = $data.ModuleToProcess }
    $nested = @($data.NestedModules | ForEach-Object { if ($_ -is [hashtable]) { $_.ModuleName } else { $_ } })
    $dlls = @(Get-ChildItem $directory.FullName -Recurse -Filter *.dll | ForEach-Object Name | Sort-Object -Unique)
    $kind = if (@($root) + $nested | Where-Object { $_ -match '\.dll$' }) { 'binary' } elseif ($root -match '\.psm1$') { 'script' } else { 'manifest' }
    '{0} {1} kind={2} root={3} nested={4} dlls_in_dir={5}' -f $directory.Name, $data.ModuleVersion, $kind, $root, ($nested -join ','), ($dlls -join ',')
}

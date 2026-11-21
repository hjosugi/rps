# Which types and members of System.Management.Automation the binary modules
# shipped with pwsh 7.7.0 actually reference: the part of the SMA facade
# (docs/06_dotnet_bridge.md, #128) those modules need. Read from metadata;
# no module is loaded.
param([string[]] $Path)
$ErrorActionPreference = 'Stop'
if (-not $Path) {
    # Every DLL of the bundled gallery modules that ship code
    # (src/Modules/PSGalleryModules.csproj upstream); those that do not
    # reference SMA report zero.
    $modules = Join-Path $PSHOME 'Modules'
    $Path = 'PSReadLine', 'Microsoft.PowerShell.ThreadJob', 'Microsoft.PowerShell.PSResourceGet', 'PackageManagement' |
        ForEach-Object { Get-ChildItem (Join-Path $modules $_) -Recurse -Filter *.dll } |
        ForEach-Object FullName
}

$allTypes = [System.Collections.Generic.HashSet[string]]::new()
$allMembers = [System.Collections.Generic.HashSet[string]]::new()
foreach ($file in $Path) {
    $stream = [System.IO.File]::OpenRead($file)
    try {
        $pe = [System.Reflection.PortableExecutable.PEReader]::new($stream)
        $reader = [System.Reflection.Metadata.PEReaderExtensions]::GetMetadataReader($pe)
        $smaReferences = @{}
        foreach ($handle in $reader.AssemblyReferences) {
            $reference = $reader.GetAssemblyReference($handle)
            if ($reader.GetString($reference.Name) -eq 'System.Management.Automation') {
                $smaReferences[[System.Reflection.Metadata.EntityHandle]$handle] = $reader.GetString($reference.Name) + ' ' + $reference.Version
            }
        }
        $typeNames = @{}
        foreach ($handle in $reader.TypeReferences) {
            $type = $reader.GetTypeReference($handle)
            $scope = $type.ResolutionScope
            # Nested types resolve through their declaring type.
            while ($scope.Kind -eq [System.Reflection.Metadata.HandleKind]::TypeReference) {
                $scope = $reader.GetTypeReference([System.Reflection.Metadata.TypeReferenceHandle]$scope).ResolutionScope
            }
            if ($smaReferences.ContainsKey($scope)) {
                $name = $reader.GetString($type.Namespace) + '.' + $reader.GetString($type.Name)
                $typeNames[[System.Reflection.Metadata.EntityHandle]$handle] = $name
            }
        }
        $members = [System.Collections.Generic.HashSet[string]]::new()
        foreach ($handle in $reader.MemberReferences) {
            $member = $reader.GetMemberReference($handle)
            $parent = $member.Parent
            if ($parent.Kind -eq [System.Reflection.Metadata.HandleKind]::TypeReference -and $typeNames.ContainsKey($parent)) {
                [void] $members.Add($typeNames[$parent] + '::' + $reader.GetString($member.Name))
            }
        }
        if ($smaReferences.Count -eq 0) { continue }
        $versions = ($smaReferences.Values | Sort-Object -Unique) -join ', '
        '{0}: references [{1}]; {2} SMA types, {3} SMA members' -f (Split-Path $file -Leaf), $versions, $typeNames.Count, $members.Count
        foreach ($name in $typeNames.Values) { [void] $allTypes.Add($name) }
        foreach ($name in $members) { [void] $allMembers.Add($name) }
    } finally {
        $stream.Dispose()
    }
}
"union: $($allTypes.Count) SMA types, $($allMembers.Count) SMA members"
'types by namespace:'
$allTypes | Group-Object { $_.Substring(0, $_.LastIndexOf('.')) } | Sort-Object Count -Descending |
    ForEach-Object { '  {0,-50} {1}' -f $_.Name, $_.Count }
'types referenced:'
$allTypes | Sort-Object | ForEach-Object { "  $_" }

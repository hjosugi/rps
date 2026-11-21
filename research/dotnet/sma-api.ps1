# The public surface of System.Management.Automation 7.7.0: what a facade
# assembly for binary modules has to expose (docs/06_dotnet_bridge.md).
$ErrorActionPreference = 'Stop'
$assembly = [System.Management.Automation.PSObject].Assembly
$types = $assembly.GetExportedTypes()
$flags = [System.Reflection.BindingFlags]'Public,Instance,Static,DeclaredOnly'
$members = foreach ($type in $types) { $type.GetMembers($flags) }
"assembly=$($assembly.GetName().Name) $($assembly.GetName().Version)"
"exported_types=$($types.Count)"
"  classes=$(@($types | Where-Object { $_.IsClass -and -not $_.IsSubclassOf([Delegate]) }).Count)"
"  interfaces=$(@($types | Where-Object IsInterface).Count)"
"  enums=$(@($types | Where-Object IsEnum).Count)"
"  structs=$(@($types | Where-Object { $_.IsValueType -and -not $_.IsEnum }).Count)"
"  delegates=$(@($types | Where-Object { $_.IsSubclassOf([Delegate]) }).Count)"
"  abstract_or_interface_with_cmdlet_base=$(@($types | Where-Object { $_.IsSubclassOf([System.Management.Automation.Cmdlet]) }).Count) subclasses of Cmdlet"
"public_members=$($members.Count)"
$members | Group-Object MemberType | Sort-Object Count -Descending | ForEach-Object { "  $($_.Name)=$($_.Count)" }
"of which enum values=$(@($members | Where-Object { $_.MemberType -eq 'Field' -and $_.DeclaringType.IsEnum }).Count)"
"of which property/event accessor methods=$(@($members | Where-Object { $_.MemberType -eq 'Method' -and $_.IsSpecialName }).Count)"
"ordinary methods=$(@($members | Where-Object { $_.MemberType -eq 'Method' -and -not $_.IsSpecialName }).Count)"
'namespaces:'
$types | Group-Object Namespace | Sort-Object Count -Descending | ForEach-Object {
    $names = @($_.Group)
    $count = @($names | ForEach-Object { $_.GetMembers($flags) }).Count
    "  {0,-60} types={1,4} members={2,5}" -f $_.Name, $_.Count, $count
}

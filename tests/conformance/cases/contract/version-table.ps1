# docs/01_compat_contract.md, "Identity". rpsh reports the version table
# of the release it reproduces, key for key. OS is left out: it names the
# machine the case runs on.
$PSVersionTable.GetType().FullName
$PSVersionTable.Keys -join ','
foreach ($key in $PSVersionTable.Keys) {
    if ($key -ne 'OS') {
        '{0} = {1} [{2}]' -f $key, ($PSVersionTable[$key] -join ' '), $PSVersionTable[$key].GetType().Name
    }
}
"PSEdition=$PSEdition IsCoreCLR=$IsCoreCLR IsLinux=$IsLinux IsMacOS=$IsMacOS IsWindows=$IsWindows"
"Host=$($Host.Name) $($Host.Version)"

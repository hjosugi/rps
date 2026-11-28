# rpsh-conformance: level=c2 LANG=C
"native-default=$PSNativeCommandUseErrorActionPreference"
$Error.Clear()
$history=$Error
sh -c 'exit 3'
"default=$?,$LASTEXITCODE,$($history.Count)"
$PSNativeCommandUseErrorActionPreference=$true
foreach ($action in 'Ignore','SilentlyContinue','Continue','Stop') {
    $ErrorActionPreference=$action
    $Error.Clear()
    try {
        $output = sh -c 'printf stdout; exit 10'
        "$action|$?|$LASTEXITCODE|$output|$($Error.Count)"
    } catch [System.Management.Automation.NativeCommandExitException] {
        "caught|$action|$?|$LASTEXITCODE|$($Error.Count)"
        $_.Exception.Message
    }
    if ($history.Count) {
        $r=$history[0]
        $r.GetType().FullName
        $r.FullyQualifiedErrorId
        $r.CategoryInfo.Category
        $r.CategoryInfo.Reason
        $r.Exception.GetType().FullName
        $r.Exception.ExitCode
        $r.Exception.ProcessId -gt 0
        $r.TargetObject -ceq (Get-Command sh).Path
        $r.Exception.Path -ceq $r.TargetObject
        $r.Exception.Message
    }
}
$ErrorActionPreference='SilentlyContinue'
$Error.Clear()
sh -c 'exit 7' | sh -c 'exit 0'
"early|$?|$LASTEXITCODE|$($Error.Count)"
sh -c 'exit 8' | sh -c 'exit 19'
"both|$?|$LASTEXITCODE|$($Error.Count)"
foreach ($item in $Error) { $item.Exception.ExitCode }
$ErrorActionPreference='Stop'
$Error.Clear()
sh -c 'printf "ordinary stderr\n" >&2; exit 0'
"stderr|$?|$LASTEXITCODE|$($Error.Count)"
try { sh -c 'exit 26' | sh -c 'exit 0' } catch {
    "stopped-early|$?|$LASTEXITCODE|$($Error.Count)|$($_.Exception.ExitCode)"
}
$Error.Clear()
$ErrorActionPreference='Inquire'
try { sh -c 'exit 2' } catch {
    "inquire|$?|$LASTEXITCODE|$($Error.Count)|$($_.FullyQualifiedErrorId)"
    $_.Exception.Message -like '*NonInteractive mode*'
}

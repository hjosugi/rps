# rpsh-conformance: level=c2
$path = mktemp
try {
    'first', 'second' > $path
    Write-Output third 1>> $path
    (Get-Content $path) -join '|'
    Write-Output abc > $path def
    (Get-Content $path) -join '|'
    Write-Output 2 > $path
    Get-Content $path
    Write-Warning warn 3>&1 > $path
    Get-Content $path
    1..3 | ForEach-Object { $_ * 2 } > $path
    (Get-Content $path) -join ','
    Get-Alias type > $path
    Get-Content $path
    sh -c 'printf "\377x\000z"' > $path
    sh -c 'printf "\001"' >> $path
    sh -c 'od -An -tx1 "$1" | tr -d " \n"' sh $path
    Write-Output ignored > $null
    sh -c 'printf ignored; exit 4' > $null
    "$?|$LASTEXITCODE"
    $result = Write-Output nothing > $path
    $null -eq $result
    Get-Content $path
} finally {
    Remove-Item $path
}

# rpsh-conformance: level=c2
$originalConsole = [Console]::OutputEncoding
$originalInput = $OutputEncoding
try {
    try { $OutputEncoding = 'utf8' } catch { $_.FullyQualifiedErrorId }
    try { Set-Variable -Name outputencoding -Value 'utf8' -ErrorAction Stop } catch { $_.FullyQualifiedErrorId }
    foreach ($encoder in @($originalInput, [Text.Encoding]::ASCII, [Text.Encoding]::UTF8, [Text.Encoding]::Unicode, [Text.Encoding]::BigEndianUnicode, [Text.Encoding]::UTF32, [Text.Encoding]::Latin1)) {
        $OutputEncoding = $encoder
        $hex = '😀Ā-é' | sh -c 'od -An -tx1 | tr -d " \n"'
        "$($encoder.WebName)=$hex"
    }
    $OutputEncoding = [Text.Encoding]::Unicode
    $hex = @() | sh -c 'od -An -tx1 | tr -d " \n"'
    "empty=$hex"
    Set-Variable -Name outputencoding -Value ([Text.Encoding]::ASCII)
    $hex = 'é' | sh -c 'od -An -tx1 | tr -d " \n"'
    "via-variable=$hex"
    Set-Item Variable:outputencoding ([Text.Encoding]::Latin1)
    $hex = 'é' | sh -c 'od -An -tx1 | tr -d " \n"'
    "via-provider=$hex"
    $OutputEncoding = $originalInput
    (Get-Alias type | cat) -join '|'
    $PSApplicationOutputEncoding = [Text.Encoding]::GetEncoding(28591)
    $latin = sh -c 'printf "\351"'
    $latin
    [Console]::OutputEncoding = [Text.Encoding]::ASCII
    $override = sh -c 'printf "\351"'
    $PSApplicationOutputEncoding = $null
    $ascii = sh -c 'printf "\351"'
    [Console]::OutputEncoding = [Text.Encoding]::Unicode
    $wide = sh -c 'printf "A\000\351\000\012\000B\000"'
    [Console]::OutputEncoding = $originalConsole
    "override=$override ascii=$ascii wide=$($wide -join '|')"
    $PSApplicationOutputEncoding = [Text.Encoding]::GetEncoding('utf-32BE')
    $codepoint = sh -c 'printf "\000\001\366\000"'
    $codepoint
    $PSApplicationOutputEncoding = $null
} finally {
    [Console]::OutputEncoding = $originalConsole
    $OutputEncoding = $originalInput
    $PSApplicationOutputEncoding = $null
}

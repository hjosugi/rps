#!/usr/bin/env pwsh
# Dumps the reference parser's tokens as `kind<TAB>start<TAB>end`.
param([Parameter(Mandatory)] [AllowEmptyString()] [string] $Source)
$tokens = $null
$errors = $null
$null = [System.Management.Automation.Language.Parser]::ParseInput($Source, [ref] $tokens, [ref] $errors)
foreach ($token in $tokens) {
    "{0}`t{1}`t{2}" -f $token.Kind, $token.Extent.StartOffset, $token.Extent.EndOffset
}

# Alias and argument-completer registration: the rest of a typical profile.
foreach ($i in 1..50) { Set-Alias "BenchAlias$i" Get-ChildItem }
foreach ($i in 1..20) {
    Register-ArgumentCompleter -CommandName "BenchFunc$i" -ScriptBlock {
        param($commandName, $wordToComplete, $cursorPosition, $ast, $fakeBoundParameters)
        @()
    }
}

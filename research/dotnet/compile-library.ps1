# Compiles research/dotnet/Probe.cs into a class library with the Roslyn
# compiler that ships with pwsh (the library twin of
# bench/startup/compile-probe.ps1). Called by research/dotnet/build.sh.
param(
    [Parameter(Mandatory)] [string] $Source,
    [Parameter(Mandatory)] [string] $Assembly
)
$ErrorActionPreference = 'Stop'

$references = @(Get-ChildItem (Join-Path $PSHOME 'ref') -Filter *.dll) +
    @(Get-Item (Join-Path $PSHOME 'System.Management.Automation.dll')) |
    ForEach-Object { [Microsoft.CodeAnalysis.MetadataReference]::CreateFromFile($_.FullName) }
$tree = [Microsoft.CodeAnalysis.CSharp.CSharpSyntaxTree]::ParseText(
    [System.IO.File]::ReadAllText($Source),
    [Microsoft.CodeAnalysis.CSharp.CSharpParseOptions]::new([Microsoft.CodeAnalysis.CSharp.LanguageVersion]::Latest))
$options = [Microsoft.CodeAnalysis.CSharp.CSharpCompilationOptions]::new(
    [Microsoft.CodeAnalysis.OutputKind]::DynamicallyLinkedLibrary).WithOptimizationLevel(
    [Microsoft.CodeAnalysis.OptimizationLevel]::Release).WithNullableContextOptions(
    [Microsoft.CodeAnalysis.NullableContextOptions]::Enable)
$compilation = [Microsoft.CodeAnalysis.CSharp.CSharpCompilation]::Create(
    [System.IO.Path]::GetFileNameWithoutExtension($Assembly), [Microsoft.CodeAnalysis.SyntaxTree[]] @($tree),
    [Microsoft.CodeAnalysis.MetadataReference[]] $references, $options)
$stream = [System.IO.File]::Create($Assembly)
try {
    $result = $compilation.Emit($stream)
} finally {
    $stream.Dispose()
}
if (-not $result.Success) {
    $result.Diagnostics | ForEach-Object { Write-Host $_.ToString() }
    Remove-Item $Assembly
    exit 1
}

# rpsh-conformance: level=c2
$original = [Console]::OutputEncoding
try {
    sh -c 'printf "\377raw"'
    [Console]::OutputEncoding = [Text.Encoding]::Unicode
    'Wide-é'
    sh -c 'printf "\000bytes"'
    [Console]::OutputEncoding = [Text.Encoding]::Latin1
    'é😀'
} finally {
    [Console]::OutputEncoding = $original
}

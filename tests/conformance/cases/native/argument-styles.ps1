# rpsh-conformance: level=c2 LANG=C
$p=Get-Variable psnativecommandargumentpassing
"$($p.GetType().FullName)|$($p.Name)|$($p.Value)|$($p.Options)|$($p.Description)"
(Get-Variable 'PSNativeCommand*' | ForEach-Object Name | Sort-Object) -join ','
"$PSNativeCommandArgumentPassing|$([int]$PSNativeCommandArgumentPassing)|$($PSNativeCommandArgumentPassing.GetType().FullName)"
foreach($style in 'Standard','Legacy','Windows') {
    $PSNativeCommandArgumentPassing=$style
    $PSNativeCommandArgumentPassing.ToString()
    ./fixtures/argv-dump
    ./fixtures/argv-dump '' x '' ''
    ./fixtures/argv-dump $null $false 0 '0' '' $null
    ./fixtures/argv-dump 'a b' ' a ' '   ' c
    ./fixtures/argv-dump 'a"b c"d' "a'b c'd"
    ./fixtures/argv-dump 'a\"b c\"d' 'a\\"b c"d'
    ./fixtures/argv-dump 'folder name\' 'folder name\\' 'ü日'
    ./fixtures/argv-dump 'one"two' 'between' 'three"four' end
    ./fixtures/argv-dump 'a""b' 'a"""b' 'x"' '"y'
    ./fixtures/argv-dump "tab`tand`ttab" "new`nline" "cr`rline"
    ./fixtures/argv-dump '-x' -w -p:$false -q:'x y' -r: $null -t:'' end
    ./fixtures/argv-dump -p: @('alpha','beta') -q:@('alpha','beta')
    ./fixtures/argv-dump @('alpha','two words','','th"ree') 'end'
    $args1=@('a b','c','"d e"','f\')
    ./fixtures/argv-dump $args1 'end'
    ./fixtures/argv-dump @('one',(1,2),'three') '.'
    ./fixtures/argv-dump -- --foo -- -w:3 '--'
    ./fixtures/argv-dump fixtures/*.txt 'fixtures/*.txt' fixtures/no-match*
    ./fixtures/argv-dump 'a\ b\' '\\"' tail
    ./fixtures/argv-dump '$x' '|' ';' '*'
    $x='sp ace'
    ./fixtures/argv-dump flag=$x -opt:$x --flag:$x
}
foreach($v in 0,1,2,'leg','Win') {
    $PSNativeCommandArgumentPassing=$v
    "$PSNativeCommandArgumentPassing|$([int]$PSNativeCommandArgumentPassing)"
}
foreach($bad in 3,'bad',$null) {
    try{$PSNativeCommandArgumentPassing=$bad} catch {
        "$($_.FullyQualifiedErrorId)|$($_.CategoryInfo)|$($_.Exception.GetType().FullName)"
        $_.Exception.Message
    }
}
[System.Management.Automation.NativeArgumentPassingStyle]::Standard.ToString()
$dir=mktemp -d
try {
    cp fixtures/argv-dump "$dir/find"
    cp fixtures/argv-dump "$dir/run.CmD"
    cp fixtures/argv-dump "$dir/tool"
    Push-Location $dir
    try {
        foreach($style in 'Standard','Windows','Legacy') {
            $PSNativeCommandArgumentPassing=$style
            "$style / selected executable"
            ./find 'a"b c"d' '' tail
            ./run.CmD 'a"b c"d' '' tail
            ./tool 'a"b c"d' '' tail
        }
    } finally { Pop-Location }
} finally { Remove-Item $dir -Recurse -Force }

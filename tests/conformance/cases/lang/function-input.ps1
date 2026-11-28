# rpsh-conformance: level=c2
function Read-Input {
    $cursor = $input
    $cursor.GetType().FullName
    [object]::ReferenceEquals($cursor, $input)
    [bool]$cursor
    $null -eq $cursor.Current
    $cursor.MoveNext()
    "first=$($input.Current)"
    foreach ($item in $input) { "next=$item"; break }
    $rest = @($cursor)
    "remaining=$($rest.Count):$rest"
    "exhausted=$input/$($cursor.MoveNext())"
    $cursor.Reset()
    $OFS = '|'
    "again=$($input -join ',')/$cursor"
    $cursor.Reset()
    $wrapped = [object[]]$input
    "wrapped=$($wrapped.Count):$wrapped"
    $cursor | ForEach-Object { "pipe=$_" }
}
10, 20, 30, 40 | Read-Input
Read-Input
function Read-Phase {
    begin { $beginCursor = $input; "begin=$input" }
    process {
        try { $beginCursor.MoveNext() } catch { $_.FullyQualifiedErrorId }
        $lastCursor = $input
        "process=$_/$input/$input"
    }
    end {
        "end=$_/$input"
        try { $lastCursor.Reset() } catch { $_.FullyQualifiedErrorId }
        try { $values = @($lastCursor) } catch { $_.FullyQualifiedErrorId }
    }
}
4, 5 | Read-Phase
filter Read-Filter { "filter=$_/$input/$input" }
'a', 'b' | Read-Filter
function End-Only { end { "$($input.Count)/$input"; $input.Reset(); "$input/$input" } }
6, 7 | End-Only
function Read-Return {
    begin { return 'begin' }
    process { return $input }
    end { return 'end' }
}
1, 2 | Read-Return
function Empty-Process { process {} end { "empty=$_/$input" } }
1, 2 | Empty-Process
function Read-MixedCase {
    $ofs = ':'
    "mixed=$InPuT"
    $INPUT.Reset()
    "cast=$([string]$Input)"
    $input.Reset()
    "count=$(@($input).Count)"
}
1, 2 | Read-MixedCase
$null | Read-MixedCase
Write-Output -InputObject $null | Read-MixedCase

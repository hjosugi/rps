# rpsh-conformance: level=c2
@() | cat
'object input' | cat
$line = sh -c 'IFS= read -r line; printf "first=<%s>\n" "$line"'
$line
cat | cat
'after native output'

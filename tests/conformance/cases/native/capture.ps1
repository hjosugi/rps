$lines = sh -c 'printf "a\nb\n"'
$lines.Count
$lines[1]
$one = sh -c 'echo single'
$one.GetType().FullName
sh -c 'echo direct'

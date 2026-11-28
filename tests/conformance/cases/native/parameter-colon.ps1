# rpsh-conformance: level=c2
/usr/bin/printf '<%s>\n' -x:$false -y: 'a b' -z:99 -w:$null -a:@('a','b') -v:'' -s: $false
/usr/bin/printf '<%s>\n' -a: @('hi','there') -b:('one two') -d: (1+1)
/usr/bin/printf '<%s>\n' -e:$null -f:@('') -g: @('') -h:@()

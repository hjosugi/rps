# rpsh-conformance: level=c2 LANG=C
$d=[ordered]@{A='a b'; Flag=$true; Empty=''; Null=$null; Items=@('aa','bb'); 'name:'=2; '--literal'='v'}
$a=@('x y',$null,'',('a','b'),'no*.txt')
function F { ./fixtures/argv-dump @args }
foreach ($style in 'Standard','Legacy','Windows') {
    $PSNativeCommandArgumentPassing=$style
    $style
    ./fixtures/argv-dump @d
    ./fixtures/argv-dump -first @a -last
    F -flag:$true -x: something -- --p z
    F 'a"b' -flag -other:2 'z"'
    $tag='hi'; ./fixtures/argv-dump @x/aa @x$tag @x-1 @x#q
    $s='a b'; ./fixtures/argv-dump before @s after
    $s=$null; ./fixtures/argv-dump before @s after
    $s=1; ./fixtures/argv-dump before @s after
    $s=@('--%','a b'); ./fixtures/argv-dump pre @s post
    $s=[pscustomobject]@{Hello=2}; ./fixtures/argv-dump @s
    $h=@{ans=3}; $o=[pscustomobject]$h; $dict=[ordered]@{ans=3}
    ./fixtures/argv-dump $h $o $dict
    ./fixtures/argv-dump @h @o @dict
    $global:s=@('aa','bb'); ./fixtures/argv-dump @global:s
    $s=@{flag=$true}; ./fixtures/argv-dump -p @s
    $s=[ordered]@{'d e'='q';'  x  '='r';'z:'='h';'-foo'='c';1=2}; ./fixtures/argv-dump @s
}

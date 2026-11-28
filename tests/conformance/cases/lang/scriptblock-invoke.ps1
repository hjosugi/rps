# rpsh-conformance: level=c2
foreach ($block in {}, { $null }, { 7 }, { 1; 2 }, { ,(3,4) }, { return 8 }, { process { 'p' } }, { end { 'e' } }) {
    $items = $block.Invoke()
    $raw = $block.InvokeReturnAsIs()
    $rawType = $null -eq $raw ? '<null>' : $raw.GetType().FullName
    "$($items.GetType().Name)|$($items.Count)|$($items -join ',')|$rawType|$(@($raw).Count)|$($raw -join ',')"
}
$s = { param($a='default', $b=3) "[$a][$b][$($args -join ',')][$($PSBoundParameters.Keys -join ',')]" }
$s.Invoke()
$s.Invoke($null)
$s.Invoke('-b', 5, 6)
$s.Invoke((1,2,3))
$s.Invoke((,(1,2,3)))
$s.Invoke('a',@(2,3))
$c = { 7 }.Invoke()
$c.Add(8)
"$($c.Count)|$($c[1].GetType().Name)|$($c.SyncRoot.GetType().Name)|$($c.ToString())"
1,2 | ForEach-Object { { "$_/$($input.GetType().Name)/$($input -join ',')" }.Invoke() }
$here = $PSCommandPath
$fileBlock = { "$($PSCommandPath -eq $here)|$($MyInvocation.MyCommand -eq $null)|$($MyInvocation.CommandOrigin)" }
$fileBlock.File -eq $here
$fileBlock.Invoke()
$memoryBlock = [scriptblock]::Create('"[$PSScriptRoot][$PSCommandPath]"')
$null -eq $memoryBlock.File
$memoryBlock.InvokeReturnAsIs()
function InvokeFixture { 42 }
$f = $function:InvokeFixture
"$($f.File -eq $here)|$($f.Equals($function:InvokeFixture))|$([object]::ReferenceEquals($f,$function:InvokeFixture))|$($f.InvokeReturnAsIs())"
$s.Equals({ 1 })
$same = [scriptblock]$s
"$($s -eq $same)|$($s -eq { 1 })|$($same -is [scriptblock])"
try { [scriptblock]'7' } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
{ 5 } | ForEach-Object InvokeReturnAsIs
$s | ForEach-Object -MemberName Invoke -ArgumentList 4,5
$item = $c.Item
"$($item.GetType().Name)|$($item.Value)|$($c.Item(0))|$([object]::ReferenceEquals($item,$item.Value))"
foreach ($failed in {throw 'bad'}, {begin{} end{}}, {clean{}}, {$null.GetType()}) {
    try { $failed.InvokeReturnAsIs('x') } catch {
        "$($_.FullyQualifiedErrorId)|$($_.Exception.GetType().Name)|$($_.Exception.InnerException.GetType().Name)|$($_.Exception.Message)"
    }
}
$global:steps = ''
foreach ($step in 1,2,3) {
    { if ($step -eq 2) { break }; $global:steps += $step }.Invoke()
}
$global:steps

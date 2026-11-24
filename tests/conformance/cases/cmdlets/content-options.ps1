# rpsh-conformance: level=c2
Get-Content fixtures/data.txt -Raw
Get-Content fixtures/data.txt -Tail 1
Get-Content fixtures/data.txt -First 1
Get-Content fixtures/data.txt -Last 1
'abc' | Out-File rpsh-out-nonl.txt -NoNewline
(Get-Content rpsh-out-nonl.txt -Raw).Length
'abc' | Out-File rpsh-out-nl.txt
(Get-Content rpsh-out-nl.txt -Raw).Length
Remove-Item rpsh-out-nonl.txt
Remove-Item rpsh-out-nl.txt

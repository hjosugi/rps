# rpsh-conformance: level=c2
(1..10 | Get-Random -SetSeed 5 -Count 3) -join ','
(1..10 | Get-Random -SetSeed 5 -Count 3) -join ','
(Get-Random -SetSeed 42 -Minimum 5 -Maximum 10)
(Get-Random -SetSeed 42 -Minimum 5 -Maximum 10)

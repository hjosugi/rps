# docs/01_compat_contract.md, "What does not repeat". An unseeded random
# number differs per run and may not be printed; a seeded sequence is the
# same in every run and is part of the answer.
$first = Get-Random -SetSeed 42 -Maximum 1000
$second = Get-Random -Maximum 1000
"$first $second"
[System.Random]::new(7).Next()
(Get-Random -SetSeed 1 -InputObject ('a', 'b', 'c', 'd', 'e') -Count 3) -join ','

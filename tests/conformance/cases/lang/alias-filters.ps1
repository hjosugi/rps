# rpsh-conformance: level=c2
Set-Alias sampleA Write-Output
Set-Alias sampleB Get-Content
Set-Alias sampleC Write-Output

(Get-Alias -Name sampleC, 'sample[ab]', sampleA | ForEach-Object Name) -join ','
(Get-Alias 'sampleC', 'sample[ab]', 'sampleA' | ForEach-Object Name) -join ','
(Get-Alias -Name 'sample*' -Exclude '*B' | ForEach-Object Name) -join ','
(Get-Alias -Exclude sampleA, sampleC -Name 'sample*' | ForEach-Object Name) -join ','
(Get-Alias -Definition Write-Output, Get-Content -Exclude '*Content' | ForEach-Object Name) -join ','
(Get-Alias -Definition Write-Output -Exclude sampleA | ForEach-Object Name) -join ','
(Get-Alias -Definition Get-Content, Write-Output, Get-Content | ForEach-Object Name) -join ','
(Get-Alias -Exclude sample* | ForEach-Object Name) -contains 'sampleA'
(Get-Alias -Exclude sample* | ForEach-Object Name) -contains 'echo'

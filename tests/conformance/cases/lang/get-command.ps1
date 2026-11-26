# rpsh-conformance: level=c2
function Local { }
(Get-Command Local).Name
(Get-Command Local).CommandType
(Get-Command Test-Path).CommandType
(Get-Command -CommandType Function).Name -contains 'Local'
(Get-Command -CommandType Cmdlet).Name -contains 'Test-Path'
(Get-Command gc).CommandType
(Get-Command -CommandType Alias | ForEach-Object Name) -contains 'gc'
(Get-Command -CommandType Alias | ForEach-Object Name) -contains 'Get-Content'
(Get-Command -CommandType Cmdlet | ForEach-Object Name) -contains 'Get-Content'
(Get-Command Get-Item, Get-Content | ForEach-Object Name | Sort-Object) -join ','
(Get-Command -Name Get-Item, Get-Content | ForEach-Object Name | Sort-Object) -join ','

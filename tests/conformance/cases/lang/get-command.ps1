# rpsh-conformance: level=c2
function Local { }
(Get-Command Local).Name
(Get-Command Local).CommandType
(Get-Command Test-Path).CommandType
(Get-Command -CommandType Function).Name -contains 'Local'
(Get-Command -CommandType Cmdlet).Name -contains 'Test-Path'

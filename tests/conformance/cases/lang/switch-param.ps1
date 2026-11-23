# rpsh-conformance: level=c2
function Show-Flag { param([switch]$On, $Name) "$On $Name" }
Show-Flag -On -Name x
Show-Flag -Name y

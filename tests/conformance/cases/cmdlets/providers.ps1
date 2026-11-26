# rpsh-conformance: level=c2
(Get-PSDrive | ForEach-Object Name | Sort-Object) -join ','
$d = Get-PSDrive -Name Env
$d.Provider.Name
(Get-PSProvider | ForEach-Object Name | Sort-Object) -join ','
(Get-PSDrive Env).Provider.Name
(Get-PSDrive Env).Name

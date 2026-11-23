# rpsh-conformance: level=c2
(Get-ChildItem fixtures/hidden -Name | Sort-Object) -join ','
(Get-ChildItem fixtures/hidden -Force -Name | Sort-Object) -join ','

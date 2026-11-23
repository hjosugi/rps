# rpsh-conformance: level=c2
('a', 'b', 'c' | Join-String -Separator ',')
([pscustomobject]@{ Name = 'x' }, [pscustomobject]@{ Name = 'y' } | Join-String -Property Name -Separator '+')

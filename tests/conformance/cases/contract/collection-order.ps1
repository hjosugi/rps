# docs/01_compat_contract.md, "What does not repeat". A Hashtable with
# string keys enumerates in a different order in every pwsh process, so no
# case may print one. Integer keys hash to themselves and the order is the
# same every time: rpsh has to lay out buckets the way .NET's Hashtable does.
$numbers = @{ 1 = 'a'; 2 = 'b'; 3 = 'c'; 10 = 'd'; 20 = 'e' }
$numbers.Keys -join ','
$ordered = [ordered]@{ gamma = 1; alpha = 2; beta = 3 }
$ordered.Keys -join ','
$set = [System.Collections.Generic.HashSet[string]]::new([string[]]('gamma', 'alpha', 'beta'))
$set -join ','
$dictionary = [System.Collections.Generic.Dictionary[string, int]]::new()
'gamma', 'alpha', 'beta' | ForEach-Object { $dictionary[$_] = 1 }
$dictionary.Keys -join ','
(('b', 'a', 'c', 'a', 'b') | Group-Object).Name -join ','
('b', 'a', 'c', 'a', 'b' | Select-Object -Unique) -join ','
([pscustomobject]@{ z = 1; a = 2; m = 3 }).psobject.Properties.Name -join ','

switch (3) { 1 { 'one' } 3 { 'three' } default { 'other' } }
switch -Wildcard ('apple') { 'a*' { 'starts with a' } '*e' { 'ends with e' } }
switch -Regex ('abc123') { '\d+' { $Matches[0] } }
switch (1, 2, 3) { { $_ -gt 1 } { "big $_" } }

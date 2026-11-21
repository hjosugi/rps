# rpsh-conformance: level=c2
$n = 7
if ($n -gt 10) { 'big' } elseif ($n -gt 5) { 'medium' } else { 'small' }
if ($null) { 'yes' } else { 'no' }
if (@()) { 'yes' } else { 'no' }
if ('0') { 'yes' } else { 'no' }
if (0) { 'yes' } else { 'no' }

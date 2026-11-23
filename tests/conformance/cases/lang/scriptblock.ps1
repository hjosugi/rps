# rpsh-conformance: level=c2
# Script block literals are values; execution is a separate engine feature.
$sb = { 1 + 1 }
$sb.GetType().FullName

# rpsh-conformance: level=c2
'{0} = {1}' -f 'a', 1
'{1}-{0}' -f 'a', 'b'
'{0:N2}' -f 1234567.891
'{0:F1}' -f 2.55
'{0:D3}' -f 7
'{0:C}' -f 12.5
'{{literal}} {0}' -f 1

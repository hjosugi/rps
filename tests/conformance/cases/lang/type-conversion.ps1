# The left operand decides the type of the operation.
1 + '2'
'1' + 2
'ab' * 3
[int]'42' + 1
[string]12 + 3
1 + 1.5
# Conversion to an integer rounds half to even.
[int]2.5
[int]3.5
[int]'0x10'
[bool]''
[bool]'false'

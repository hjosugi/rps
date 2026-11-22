# rpsh-conformance: level=c2
# Double-quoted escapes, here-strings, typographic quotes and CR/LF content.
"tab`there"
"newline`nhere"
"dollar `$x"
"unicode `u{1F600}"
"quote `" inside"
"double ""quote"""
@"
line 1
line 2 $(1 + 1)
"@
@'
literal $x
line 2
'@
‘single’
“double”

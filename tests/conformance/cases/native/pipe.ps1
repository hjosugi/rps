# rpsh-conformance: level=c2
sh -c 'printf "c\nb\na\n"' | sort
'x', 'y' | sh -c 'cat'
sh -c 'printf "a\nb\n"' | tr a-z A-Z | ForEach-Object { "[$_]" }

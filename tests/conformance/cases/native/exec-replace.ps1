# rpsh-conformance: level=c2 LANG=C
$env:RPSH_EXEC_EXPECTED_PID=[string]$PID
$env:RPSH_EXEC_SENTINEL='a b=ü'
$env:RPSH_EXEC_GONE=$null
'before exec'
exec sh -c 'if test "$$" = "$RPSH_EXEC_EXPECTED_PID"; then echo same-pid; fi; printf "argv0=%s\nenv=%s\ngone=%s\n" "$0" "$RPSH_EXEC_SENTINEL" "${RPSH_EXEC_GONE-unset}"; printf "<%s>\n" "$@"; echo stderr >&2; exit 19' child 'a b' '' -w 2 -flag:$true -- '--two' '2>&1'
'unreachable'

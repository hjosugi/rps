# rpsh-conformance: level=c2 LANG=C
'before'
try {
    throw 'sample failure'
} finally { 'cleanup' }
'unreachable'

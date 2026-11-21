# rpsh-conformance: level=c2
# pwsh expands wildcards for native commands on Unix, unless quoted.
sh -c 'echo "$@"' sh fixtures/*.txt
sh -c 'echo "$@"' sh 'fixtures/*.txt'
sh -c 'echo "$#"' sh fixtures/none-*.txt

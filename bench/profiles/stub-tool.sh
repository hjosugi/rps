#!/bin/sh
# A deterministic stand-in for "tool init": 50 function definitions.
i=1
while [ "$i" -le 50 ]; do
    printf 'function BenchToolFunc%s { param($x) $x + %s }\n' "$i" "$i"
    i=$((i + 1))
done

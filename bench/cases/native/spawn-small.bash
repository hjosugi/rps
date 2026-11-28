# Use the external true executable for each iteration.
enable -n true
failures=0
for ((i = 0; i < 10; i++)); do
    true || ((failures++))
done
printf '%d\n' "$failures"

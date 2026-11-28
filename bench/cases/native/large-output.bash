count=0
while IFS= read -r line; do
    ((count++))
done < <(seq 1 200000)
printf '%d\n' "$count"

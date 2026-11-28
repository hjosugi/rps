seq 1 20000 | wc -l
count=0
while IFS= read -r line; do
    ((count++))
done < <(seq 1 5000)
printf '%d\n' "$count"

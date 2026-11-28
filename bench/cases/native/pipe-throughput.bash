seq 1 200000 | wc -l
count=0
while IFS= read -r line; do
    ((count++))
done < <(seq 1 50000)
printf '%d\n' "$count"
sum=0
while IFS= read -r line; do
    ((sum += ${#line}))
done < <(seq 1 50000)
printf '%d\n' "$sum"

# Eight MiB cross two native pipes, including values that are not UTF-8.
head -c 8388608 /dev/zero | tr '\000' '\377' | wc -c

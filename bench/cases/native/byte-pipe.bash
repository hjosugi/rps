head -c 8388608 /dev/zero | tr '\000' '\377' | wc -c

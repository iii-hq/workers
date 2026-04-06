#!/bin/bash

mkdir -p /logs/verifier

if [ ! -f /task/fizzbuzz.py ]; then
    echo 0 > /logs/verifier/reward.txt
    echo "FAIL: fizzbuzz.py not found"
    exit 0
fi

ACTUAL=$(cd /task && python3 fizzbuzz.py 2>&1) || true
LINES=$(echo "$ACTUAL" | wc -l | tr -d ' ')

PASS=1

if [ "$LINES" != "100" ]; then
    echo "FAIL: expected 100 lines, got $LINES"
    PASS=0
fi

for i in $(seq 1 100); do
    LINE=$(echo "$ACTUAL" | sed -n "${i}p" | tr -d '[:space:]')
    if [ $((i % 15)) -eq 0 ]; then
        EXPECTED="FizzBuzz"
    elif [ $((i % 3)) -eq 0 ]; then
        EXPECTED="Fizz"
    elif [ $((i % 5)) -eq 0 ]; then
        EXPECTED="Buzz"
    else
        EXPECTED="$i"
    fi
    if [ "$LINE" != "$EXPECTED" ]; then
        echo "FAIL: line $i should be '$EXPECTED', got '$LINE'"
        PASS=0
    fi
done

echo $PASS > /logs/verifier/reward.txt
[ "$PASS" = "1" ] && echo "PASS: all 100 lines correct" || echo "FAIL: some checks failed"

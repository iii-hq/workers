#!/bin/bash
set -e

mkdir -p /logs/verifier

if [ ! -f /task/fizzbuzz.py ]; then
    echo 0 > /logs/verifier/reward.txt
    echo "FAIL: fizzbuzz.py not found"
    exit 0
fi

ACTUAL=$(cd /task && python3 fizzbuzz.py 2>/dev/null)
LINES=$(echo "$ACTUAL" | wc -l | tr -d ' ')

PASS=1

if [ "$LINES" != "100" ]; then
    echo "FAIL: expected 100 lines, got $LINES"
    PASS=0
fi

LINE1=$(echo "$ACTUAL" | sed -n '1p')
LINE3=$(echo "$ACTUAL" | sed -n '3p')
LINE5=$(echo "$ACTUAL" | sed -n '5p')
LINE15=$(echo "$ACTUAL" | sed -n '15p')

[ "$LINE1" = "1" ] || { echo "FAIL: line 1 should be '1', got '$LINE1'"; PASS=0; }
[ "$LINE3" = "Fizz" ] || { echo "FAIL: line 3 should be 'Fizz', got '$LINE3'"; PASS=0; }
[ "$LINE5" = "Buzz" ] || { echo "FAIL: line 5 should be 'Buzz', got '$LINE5'"; PASS=0; }
[ "$LINE15" = "FizzBuzz" ] || { echo "FAIL: line 15 should be 'FizzBuzz', got '$LINE15'"; PASS=0; }

echo $PASS > /logs/verifier/reward.txt
[ "$PASS" = "1" ] && echo "PASS: all checks passed" || echo "FAIL: some checks failed"

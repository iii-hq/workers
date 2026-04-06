#!/bin/bash
set -e
mkdir -p /logs/verifier

SCORE=0
TOTAL=7

check() {
    if [ -f "/task/$1" ]; then
        SCORE=$((SCORE + 1))
        echo "PASS: $1 exists"
    else
        echo "FAIL: $1 missing"
    fi
}

check "documents/report.pdf"
check "images/photo.jpg"
check "images/image.png"
check "text/notes.txt"
check "text/readme.txt"
check "data/data.csv"
check "data/budget.csv"

if [ "$SCORE" -eq "$TOTAL" ]; then
    echo 1 > /logs/verifier/reward.txt
else
    echo 0 > /logs/verifier/reward.txt
fi

echo "Score: $SCORE/$TOTAL"

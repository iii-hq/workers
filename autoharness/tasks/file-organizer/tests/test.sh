#!/bin/bash
set -e
mkdir -p /logs/verifier

SCORE=0
TOTAL=14

check_exists() {
    if [ -f "/task/$1" ]; then
        SCORE=$((SCORE + 1))
        echo "PASS: $1 exists"
    else
        echo "FAIL: $1 missing"
    fi
}

check_absent() {
    if [ ! -f "/task/$1" ]; then
        SCORE=$((SCORE + 1))
        echo "PASS: $1 removed from root"
    else
        echo "FAIL: $1 still exists in root"
    fi
}

check_exists "documents/report.pdf"
check_exists "images/photo.jpg"
check_exists "images/image.png"
check_exists "text/notes.txt"
check_exists "text/readme.txt"
check_exists "data/data.csv"
check_exists "data/budget.csv"

check_absent "report.pdf"
check_absent "photo.jpg"
check_absent "image.png"
check_absent "notes.txt"
check_absent "readme.txt"
check_absent "data.csv"
check_absent "budget.csv"

if [ "$SCORE" -eq "$TOTAL" ]; then
    echo 1 > /logs/verifier/reward.txt
else
    echo 0 > /logs/verifier/reward.txt
fi

echo "Score: $SCORE/$TOTAL"

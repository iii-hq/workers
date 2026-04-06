#!/bin/bash

mkdir -p /logs/verifier

if [ ! -f /task/hello.txt ]; then
    echo 0 > /logs/verifier/reward.txt
    echo "FAIL: hello.txt not found"
    exit 0
fi

EXPECTED_FILE=$(mktemp)
printf 'Hello, World!\n' > "$EXPECTED_FILE"

if cmp -s /task/hello.txt "$EXPECTED_FILE"; then
    echo 1 > /logs/verifier/reward.txt
    echo "PASS: hello.txt contains correct content (byte-exact)"
else
    echo 0 > /logs/verifier/reward.txt
    echo "FAIL: hello.txt does not match expected content"
    echo "Expected bytes:"
    xxd "$EXPECTED_FILE" | head -3
    echo "Actual bytes:"
    xxd /task/hello.txt | head -3
fi

rm -f "$EXPECTED_FILE"

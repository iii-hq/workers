#!/bin/bash
set -e

EXPECTED="Hello, World!"
ACTUAL=$(cat /task/hello.txt 2>/dev/null || echo "FILE_NOT_FOUND")

if [ "$ACTUAL" = "$EXPECTED" ]; then
    echo 1 > /logs/verifier/reward.txt
    echo "PASS: hello.txt contains correct content"
else
    echo 0 > /logs/verifier/reward.txt
    echo "FAIL: expected '$EXPECTED', got '$ACTUAL'"
fi

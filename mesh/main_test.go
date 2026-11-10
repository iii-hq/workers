package main

import (
	"context"
	"strings"
	"testing"
)

func TestRetiredCommandsAreRefused(t *testing.T) {
	for _, args := range [][]string{{"join", "https://h", "ABCD-EFGH"}, {"join", "--token", "x"}, {"leave"}, {"node"}, {"status"}} {
		if err := run(context.Background(), args); err == nil || !strings.Contains(err.Error(), "unexpected positional arguments") {
			t.Fatalf("%v: %v", args, err)
		}
	}
}

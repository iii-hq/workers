package main

import (
	"context"
	"io"
	"strings"
	"testing"
)

func TestCommandDispatchValidatesBeforeElevation(t *testing.T) {
	for _, tc := range []struct {
		args []string
		want string
	}{{[]string{"join", "https://h"}, "usage: mesh join"}, {[]string{"leave", "extra"}, "usage: mesh leave"}, {[]string{"node", "extra"}, "usage: mesh node"}, {[]string{"status", "extra"}, "usage: mesh status"}} {
		if err := run(context.Background(), tc.args, io.Discard); err == nil || !strings.Contains(err.Error(), tc.want) {
			t.Fatalf("%v: %v", tc.args, err)
		}
	}
}

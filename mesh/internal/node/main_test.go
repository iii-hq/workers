package node

import (
	"os"
	"testing"
)

// Current SDK telemetry opens a second WebSocket. Protocol fixtures model one
// worker connection, so keep test-only telemetry out of their wire assertions.
func TestMain(m *testing.M) {
	os.Setenv("OTEL_ENABLED", "false")
	os.Exit(m.Run())
}

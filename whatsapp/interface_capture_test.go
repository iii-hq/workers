package main

import (
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/coder/websocket"
)

// Exercise the real CLI in a subprocess so its flags and signal handling do not
// interfere with the test runner. Only the engine transport is replaced.
func TestInterfaceCaptureSubprocess(t *testing.T) {
	if os.Getenv("WHATSAPP_INTERFACE_CAPTURE_TEST") != "1" {
		return
	}
	flag.CommandLine = flag.NewFlagSet("whatsapp", flag.ExitOnError)
	os.Args = []string{"whatsapp", "--collect-interface", "--url", os.Getenv("WHATSAPP_INTERFACE_CAPTURE_URL"), "--config", os.Getenv("WHATSAPP_INTERFACE_CAPTURE_CONFIG")}
	if err := run(); err != nil {
		t.Fatal(err)
	}
}

func TestInterfaceCaptureRegistersWithoutBackendsOrStorage(t *testing.T) {
	clearCloudEnvironment(t)
	frames := make(chan map[string]json.RawMessage, 1024)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		for {
			_, body, err := conn.Read(r.Context())
			if err != nil {
				return
			}
			var frame map[string]json.RawMessage
			if json.Unmarshal(body, &frame) != nil {
				return
			}
			frames <- frame
			if string(frame["type"]) == `"invokefunction"` && len(frame["invocation_id"]) > 0 {
				reply := callbackInvocationReply(frame)
				reply["error"] = map[string]string{"code": "function_not_found", "message": "capture has no backend"}
				encoded, _ := json.Marshal(reply)
				if conn.Write(r.Context(), websocket.MessageText, encoded) != nil {
					return
				}
			}
		}
	}))
	defer server.Close()
	dir := t.TempDir()
	dataDir := filepath.Join(dir, "existing-data")
	if err := os.Mkdir(dataDir, 0700); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"unexpected.txt", ".development-tunnel.json"} {
		if err := os.WriteFile(filepath.Join(dataDir, name), []byte("existing data must not be inspected or modified"), 0600); err != nil {
			t.Fatal(err)
		}
	}
	seed := filepath.Join(dir, "collect.yaml")
	if err := os.WriteFile(seed, []byte("data_dir: "+dataDir+"\n"), 0600); err != nil {
		t.Fatal(err)
	}
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, executable, "-test.run=^TestInterfaceCaptureSubprocess$")
	cmd.Env = append(os.Environ(), "WHATSAPP_INTERFACE_CAPTURE_TEST=1", "WHATSAPP_INTERFACE_CAPTURE_URL=ws"+strings.TrimPrefix(server.URL, "http"), "WHATSAPP_INTERFACE_CAPTURE_CONFIG="+seed)
	var output bytes.Buffer
	cmd.Stdout, cmd.Stderr = &output, &output
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	finished := false
	defer func() {
		if !finished {
			_ = cmd.Process.Kill()
			<-done
		}
	}()
	want := make(map[string]bool)
	for _, entry := range NewRegistry(nil).Catalog() {
		want[entry.ID] = true
	}
	for _, id := range []string{"whatsapp::configuration-id", "whatsapp::config-status", "whatsapp::on-config-change", "whatsapp::ui-content", "whatsapp::cloud::webhook::verify", "whatsapp::cloud::webhook::receive", "whatsapp::cloud::flows::endpoint", "whatsapp::cloud::tunnel::changed"} {
		want[id] = true
	}
	for len(want) > 0 {
		select {
		case frame := <-frames:
			if string(frame["type"]) == `"invokefunction"` && string(frame["function_id"]) != `"engine::workers::register"` {
				t.Fatalf("interface capture invoked a backend: %s", frame["function_id"])
			}
			if string(frame["type"]) == `"registerfunction"` {
				var id string
				if err := json.Unmarshal(frame["id"], &id); err != nil {
					t.Fatal(err)
				}
				delete(want, id)
			}
		case err := <-done:
			finished = true
			t.Fatalf("capture exited before registering %d functions: %v\n%s", len(want), err, output.String())
		case <-ctx.Done():
			t.Fatalf("capture did not register %d functions: %v", len(want), ctx.Err())
		}
	}
	if err := cmd.Process.Signal(syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	err = <-done
	finished = true
	if err != nil {
		t.Fatalf("capture did not shut down cleanly: %v\n%s", err, output.String())
	}
	for len(frames) > 0 {
		frame := <-frames
		if string(frame["type"]) == `"invokefunction"` && string(frame["function_id"]) != `"engine::workers::register"` {
			t.Fatalf("interface capture invoked a backend: %s", frame["function_id"])
		}
	}
	entries, err := os.ReadDir(dataDir)
	if err != nil || len(entries) != 2 {
		t.Fatalf("capture changed existing storage: %v (%d files)", err, len(entries))
	}
}

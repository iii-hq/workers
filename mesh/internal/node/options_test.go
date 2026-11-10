package node

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/iii-hq/workers/mesh/internal/drives"
)

func TestURLOverrideKeepsGatewayGateAndDropsToken(t *testing.T) {
	t.Setenv("III_TOKEN", "local-engine-token")
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	auth := make(chan []string, 8)
	server := httptest.NewServer(http.HandlerFunc(func(rw http.ResponseWriter, r *http.Request) {
		auth <- r.Header.Values("Authorization")
		c, err := websocket.Accept(rw, r, nil)
		if err != nil {
			return
		}
		defer c.CloseNow()
		for {
			if _, _, err := c.Read(ctx); err != nil {
				return
			}
		}
	}))
	defer server.Close()
	override := "ws" + strings.TrimPrefix(server.URL, "http")
	n, k, m, w := testNodeWith(t, Options{URL: override})
	m.port = 0
	apply, _ := Sign(k, "mesh-node::workers::apply", WorkersRequest{Namespace: "project", Workers: []Worker{}}, time.Now())
	if _, err := n.handle(ctx, "mesh-node::workers::apply", apply); err == nil {
		t.Fatal("override bypassed the authenticated gateway gate")
	}
	done := make(chan error, 1)
	go func() { done <- n.Run(ctx) }()
	select {
	case <-auth:
		t.Fatal("connected without authenticated gateway")
	case <-time.After(40 * time.Millisecond):
	}
	m.setPort(50222)
	select {
	case got := <-auth:
		if len(got) != 0 {
			t.Fatalf("Authorization leaked to hub: %q", got)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	apply, _ = Sign(k, "mesh-node::workers::apply", WorkersRequest{Namespace: "project", Workers: []Worker{}}, time.Now())
	if _, err := n.handle(ctx, "mesh-node::workers::apply", apply); err != nil {
		t.Fatal(err)
	}
	w.mu.Lock()
	got := w.runtime
	w.mu.Unlock()
	if got != (Runtime{Node: "laptop-2", Namespace: "project", URL: override}) {
		t.Fatalf("runtime %+v", got)
	}
	cancel()
	if err := <-done; err != nil {
		t.Fatal(err)
	}
}

func TestDisableUpdateRefusesAndKeepsBinary(t *testing.T) {
	binary := filepath.Join(t.TempDir(), "mesh")
	os.WriteFile(binary, []byte("old"), 0755)
	content := []byte("replacement")
	served := false
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { served = true; w.Write(content) }))
	defer server.Close()
	n, k, _, _ := testNodeWith(t, Options{BinaryPath: binary, HTTPClient: server.Client(), DisableUpdate: true})
	s, _ := Sign(k, "mesh-node::update", UpdateRequest{Version: "0.2.0", ArtifactURL: server.URL, SHA256: fmt.Sprintf("%x", sha256.Sum256(content))}, time.Now())
	if _, err := n.handle(t.Context(), "mesh-node::update", s); err == nil || !strings.Contains(err.Error(), "does not self-update") {
		t.Fatalf("update not refused: %v", err)
	}
	if got, _ := os.ReadFile(binary); string(got) != "old" || served {
		t.Fatalf("binary touched: %q served=%v", got, served)
	}
	if n.WantedVersion() != "0.2.0" {
		t.Fatalf("wanted version %q", n.WantedVersion())
	}
	select {
	case <-n.restart:
		t.Fatal("refused update requested a restart")
	default:
	}
	s, _ = Sign(k, "mesh-node::workers::apply", WorkersRequest{Namespace: "project", Workers: []Worker{}}, time.Now())
	if _, err := n.handle(t.Context(), "mesh-node::workers::apply", s); err != nil || n.WantedVersion() != "" {
		t.Fatalf("apply after matching versions kept wanted version %q: %v", n.WantedVersion(), err)
	}
}

func TestUnsupportedDrivesAnswerWithoutErrors(t *testing.T) {
	n, k, _, _ := testNodeWith(t, Options{Drives: &UnsupportedDrives{}})
	desired := drives.Desired{Serve: []drives.Export{{Name: "share"}}, Mount: []drives.Mount{{Name: "proj"}}}
	s, _ := Sign(k, "mesh-node::drives::apply", desired, time.Now())
	raw, err := n.handle(t.Context(), "mesh-node::drives::apply", s)
	if err != nil {
		t.Fatal(err)
	}
	var result []DriveStatus
	json.Unmarshal(raw, &result)
	if len(result) != 2 || !reflect.DeepEqual(n.Status().Drives, result) {
		t.Fatalf("result %s status %+v", raw, n.Status().Drives)
	}
	for _, r := range result {
		if r.State != "unsupported" || r.Error != "" {
			t.Fatalf("drive must report unsupported without error: %s", raw)
		}
	}
	s, _ = Sign(k, "mesh-node::drives::apply", drives.Desired{}, time.Now())
	if raw, err = n.handle(t.Context(), "mesh-node::drives::apply", s); err != nil || string(raw) != "[]" || len(n.Status().Drives) != 0 {
		t.Fatalf("empty desired: %s %v %+v", raw, err, n.Status().Drives)
	}
}

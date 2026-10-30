//go:build linux || darwin

package node

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"os/user"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/supervisor"
)

func TestSupervisorAdapterSignedDesiredStateAndRuntime(t *testing.T) {
	if os.Getuid() == 0 {
		t.Skip("same-user integration requires a non-root test runner")
	}
	u, err := user.Current()
	if err != nil {
		t.Fatal(err)
	}
	op, err := supervisor.LookupOperator(u.Username)
	if err != nil {
		t.Fatal(err)
	}
	cache := filepath.Join(t.TempDir(), "cache")
	if err := os.Mkdir(cache, 0755); err != nil {
		t.Fatal(err)
	}
	script := []byte("#!/bin/sh\necho \"$III_URL $III_NAMESPACE $III_CONFIG_NAME\"\necho stderr-ready >&2\nexec sleep 60\n")
	digest := fmt.Sprintf("%x", sha256.Sum256(script))
	if err := os.WriteFile(filepath.Join(cache, digest), script, 0555); err != nil {
		t.Fatal(err)
	}
	s := supervisor.New(cache, filepath.Join(t.TempDir(), "logs"), op, "", "")
	defer s.Close()
	n, key, network, _ := testNode(t)
	n.workers = NewWorkerController(s)
	apply := func(workers ...Worker) []WorkerResult {
		t.Helper()
		signed, err := Sign(key, "mesh-node::workers::apply", WorkersRequest{Namespace: "project", Workers: workers}, time.Now())
		if err != nil {
			t.Fatal(err)
		}
		raw, err := n.handle(context.Background(), "mesh-node::workers::apply", signed)
		if err != nil {
			t.Fatal(err)
		}
		var results []WorkerResult
		if err = json.Unmarshal(raw, &results); err != nil {
			t.Fatal(err)
		}
		return results
	}
	wait := func(want string) {
		t.Helper()
		deadline := time.Now().Add(3 * time.Second)
		for time.Now().Before(deadline) {
			for _, st := range n.Status().Workers {
				if strings.Contains(st.LogTail, want) && strings.Contains(st.LogTail, "stderr-ready") {
					return
				}
			}
			time.Sleep(10 * time.Millisecond)
		}
		t.Fatalf("logs missing %s: %+v", want, n.Status().Workers)
	}
	old := Worker{Name: "old", Version: "1", ArtifactURL: "https://unused.invalid/artifact", SHA256: digest}
	if rs := apply(old); len(rs) != 1 || rs[0].Error != "" {
		t.Fatalf("initial apply: %+v", rs)
	}
	wait("ws://hub.iii.internal:50222 project laptop-2__old")
	first := s.Status()[0].PID
	valid := old
	valid.Name = "valid"
	failed := old
	failed.ResolutionError = "no artifact for target"
	if rs := apply(failed, valid); len(rs) != 2 {
		t.Fatalf("results: %+v", rs)
	}
	wait("ws://hub.iii.internal:50222 project laptop-2__valid")
	if got := s.Status(); len(got) != 1 || got[0].Name != "valid" || got[0].PID == first {
		t.Fatalf("supervisor retained unresolved worker: %+v", got)
	}
	statuses := n.Status().Workers
	if len(statuses) != 2 {
		t.Fatalf("overlay statuses: %+v", statuses)
	}
	for _, st := range statuses {
		if st.Name == "old" && (st.State != "error" || st.Error != "no artifact for target") {
			t.Fatalf("lost resolution overlay: %+v", st)
		}
	}
	network.setPort(50333)
	if err := n.setRuntime(context.Background(), "project"); err != nil {
		t.Fatal(err)
	}
	wait("ws://hub.iii.internal:50333 project laptop-2__valid")
	if rs := apply(failed); len(rs) != 1 || rs[0].Error == "" {
		t.Fatalf("all unresolved: %+v", rs)
	}
	if len(s.Status()) != 0 {
		t.Fatalf("empty runnable set retained children: %+v", s.Status())
	}
	if rs := apply(); len(rs) != 0 || len(n.Status().Workers) != 0 {
		t.Fatalf("empty desired set retained errors: %+v", n.Status().Workers)
	}
}

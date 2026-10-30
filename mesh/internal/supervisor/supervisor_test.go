//go:build linux || darwin

package supervisor

import (
	"context"
	"crypto/sha256"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/user"
	"path/filepath"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestWorkerEnv(t *testing.T) {
	want := []string{"III_URL=ws://hub.iii.internal:49135", "III_NAMESPACE=proj", "III_WORKER_NAME=ide", "III_CONFIG_NAME=laptop__ide"}
	if got := WorkerEnv("laptop", "proj", "ide"); !reflect.DeepEqual(got, want) {
		t.Fatalf("environment = %v", got)
	}
}
func TestBackoff(t *testing.T) {
	for i, seconds := range []int{1, 2, 4, 8, 16, 32, 60, 60} {
		if got := Backoff(i); got != time.Duration(seconds)*time.Second {
			t.Fatalf("restart %d: %v", i, got)
		}
	}
	if Backoff(1000) != time.Minute || Backoff(-1) != time.Second {
		t.Fatal("backoff must saturate without overflow")
	}
}
func testSupervisor(t *testing.T) *Supervisor {
	t.Helper()
	if os.Getuid() == 0 {
		t.Skip("same-user lifecycle tests require a non-root test runner; privileged boundary has a separate test")
	}
	u, err := user.Current()
	if err != nil {
		t.Fatal(err)
	}
	op, err := LookupOperator(u.Username)
	if err != nil {
		t.Fatal(err)
	}
	s := New(filepath.Join(t.TempDir(), "cache"), filepath.Join(t.TempDir(), "logs"), op, "laptop", "proj")
	t.Cleanup(func() {
		if err := s.Close(); err != nil {
			t.Error(err)
		}
	})
	return s
}
func artifact(t *testing.T, s *Supervisor, script string) (Desired, *atomic.Int32) {
	t.Helper()
	count := new(atomic.Int32)
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) { count.Add(1); fmt.Fprint(w, script) }))
	t.Cleanup(server.Close)
	s.client = server.Client()
	return Desired{Name: "ide", Version: "1.0.0", URL: server.URL, SHA256: fmt.Sprintf("%x", sha256.Sum256([]byte(script)))}, count
}
func awaitStatus(t *testing.T, s *Supervisor, check func(Status) bool) Status {
	t.Helper()
	deadline := time.Now().Add(6 * time.Second)
	for time.Now().Before(deadline) {
		for _, st := range s.Status() {
			if check(st) {
				return st
			}
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("status did not converge: %+v", s.Status())
	return Status{}
}
func applyOK(t *testing.T, s *Supervisor, ds ...Desired) {
	t.Helper()
	rs := s.Apply(context.Background(), ds)
	if len(rs) != len(ds) {
		t.Fatalf("results = %+v", rs)
	}
	for _, r := range rs {
		if r.Error != "" {
			t.Fatalf("apply %s: %s", r.Name, r.Error)
		}
	}
}
func TestShaMismatchRefusesStart(t *testing.T) {
	s := testSupervisor(t)
	marker := filepath.Join(t.TempDir(), "started")
	d, _ := artifact(t, s, "#!/bin/sh\ntouch '"+marker+"'\n")
	d.SHA256 = strings.Repeat("a", 64)
	rs := s.Apply(context.Background(), []Desired{d})
	if len(rs) != 1 || !strings.Contains(rs[0].Error, d.SHA256) {
		t.Fatalf("mismatch = %+v", rs)
	}
	if _, err := os.Stat(marker); !os.IsNotExist(err) {
		t.Fatalf("unverified worker executed: %v", err)
	}
	if got := s.Status(); len(got) != 1 || got[0].State != "error" {
		t.Fatalf("status = %+v", got)
	}
	entries, err := os.ReadDir(s.cacheDir)
	if err != nil || len(entries) != 0 {
		t.Fatalf("partial cache: %v %v", entries, err)
	}
}
func TestStopsUndesired(t *testing.T) {
	s := testSupervisor(t)
	dir := t.TempDir()
	d, _ := artifact(t, s, "#!/bin/sh\ntrap 'echo stopped > "+dir+"/stopped; exit 0' TERM\necho ready\nwhile :; do sleep 1; done\n")
	applyOK(t, s, d)
	awaitStatus(t, s, func(st Status) bool { return strings.Contains(st.LogTail, "ready") })
	applyOK(t, s)
	if len(s.Status()) != 0 {
		t.Fatalf("removed status retained: %+v", s.Status())
	}
	if _, err := os.Stat(filepath.Join(dir, "stopped")); err != nil {
		t.Fatalf("worker did not terminate: %v", err)
	}
}

func TestRuntimeReplacesEnvironmentAndSuspends(t *testing.T) {
	s := testSupervisor(t)
	t.Setenv("III_URL", "ws://ambient.invalid")
	t.Setenv("MESH_PRIVATE_TEST", "must-not-inherit")
	script := "#!/bin/sh\nprintf 'pid=%s url=%s namespace=%s name=%s config=%s secret=%s home=%s user=%s uid=%s\\n' \"$$\" \"$III_URL\" \"$III_NAMESPACE\" \"$III_WORKER_NAME\" \"$III_CONFIG_NAME\" \"${MESH_PRIVATE_TEST-unset}\" \"$HOME\" \"$USER\" \"$(id -u)\"\nexec sleep 60\n"
	d, count := artifact(t, s, script)
	ctx, cancel := context.WithCancel(context.Background())
	if rs := s.Apply(ctx, []Desired{d}); len(rs) != 1 || rs[0].Error != "" {
		t.Fatalf("apply = %+v", rs)
	}
	cancel()
	first := awaitStatus(t, s, func(st Status) bool { return strings.Contains(st.LogTail, "config=laptop__ide") })
	if !strings.Contains(first.LogTail, "secret=unset") || !strings.Contains(first.LogTail, fmt.Sprintf("uid=%d", os.Getuid())) {
		t.Fatalf("unsafe process environment: %+v", first)
	}
	rt := Runtime{Node: "assigned-2", Namespace: "project-2", URL: "ws://hub.iii.internal:50000"}
	if err := s.SetRuntime(context.Background(), rt); err != nil {
		t.Fatal(err)
	}
	second := awaitStatus(t, s, func(st Status) bool {
		return strings.Contains(st.LogTail, "url=ws://hub.iii.internal:50000 namespace=project-2 name=ide config=assigned-2__ide")
	})
	if second.PID == first.PID {
		t.Fatal("runtime change did not replace process")
	}
	if err := s.SetRuntime(context.Background(), rt); err != nil {
		t.Fatal(err)
	}
	if s.Status()[0].PID != second.PID {
		t.Fatal("idempotent runtime restarted process")
	}
	rt.URL = ""
	if err := s.SetRuntime(context.Background(), rt); err != nil {
		t.Fatal(err)
	}
	if st := s.Status()[0]; st.State != "suspended" || st.PID != 0 {
		t.Fatalf("suspended = %+v", st)
	}
	rt.URL = "ws://hub.iii.internal:50001"
	if err := s.SetRuntime(context.Background(), rt); err != nil {
		t.Fatal(err)
	}
	awaitStatus(t, s, func(st Status) bool { return st.PID != 0 && strings.Contains(st.LogTail, ":50001") })
	if count.Load() != 1 {
		t.Fatalf("cache miss across restarts: %d downloads", count.Load())
	}
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	if len(s.Status()) != 0 {
		t.Fatal("closed supervisor retained desired workers")
	}
	if rs := s.Apply(context.Background(), []Desired{d}); len(rs) != 1 || rs[0].Error == "" {
		t.Fatalf("closed apply = %+v", rs)
	}
}
func TestBootstrapWaitsForAuthenticatedRuntime(t *testing.T) {
	s := testSupervisor(t)
	s.runtime = Runtime{}
	d, count := artifact(t, s, "#!/bin/sh\necho authenticated\nexec sleep 60\n")
	applyOK(t, s, d)
	if st := s.Status()[0]; st.State != "suspended" || st.PID != 0 || count.Load() != 0 {
		t.Fatalf("bootstrap started early: %+v", st)
	}
	if err := s.SetRuntime(context.Background(), Runtime{Node: "assigned", Namespace: "project", URL: "ws://hub.iii.internal:50000"}); err != nil {
		t.Fatal(err)
	}
	awaitStatus(t, s, func(st Status) bool { return strings.Contains(st.LogTail, "authenticated") })
}
func TestRestartsAfterExitWithBackoffAndCloseCancelsRetry(t *testing.T) {
	s := testSupervisor(t)
	d, _ := artifact(t, s, "#!/bin/sh\necho crashed\nexit 3\n")
	start := time.Now()
	applyOK(t, s, d)
	awaitStatus(t, s, func(st Status) bool { return st.State == "backoff" && st.Restarts == 0 })
	if time.Since(start) >= time.Second {
		t.Fatal("initial exit took too long")
	}
	st := awaitStatus(t, s, func(st Status) bool { return st.Restarts == 1 && st.State == "backoff" })
	if time.Since(start) < time.Second || strings.Count(st.LogTail, "crashed") != 2 {
		t.Fatalf("restart delay/log = %+v", st)
	}
	start = time.Now()
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	if time.Since(start) > time.Second {
		t.Fatal("Close waited for retry delay")
	}
}
func TestCompleteDesiredSetIndependentFailuresAndReplacement(t *testing.T) {
	s := testSupervisor(t)
	old, _ := artifact(t, s, "#!/bin/sh\necho old\nexec sleep 60\n")
	applyOK(t, s, old)
	first := awaitStatus(t, s, func(st Status) bool { return strings.Contains(st.LogTail, "old") })
	applyOK(t, s, old)
	if s.Status()[0].PID != first.PID {
		t.Fatal("identical apply restarted worker")
	}
	next, _ := artifact(t, s, "#!/bin/sh\necho new\nexec sleep 60\n")
	next.Name = "valid"
	bad := next
	bad.Name = "missing"
	bad.SHA256 = strings.Repeat("b", 64)
	rs := s.Apply(context.Background(), []Desired{bad, next})
	if len(rs) != 2 || rs[0].Error == "" || rs[1].Error != "" {
		t.Fatalf("results = %+v", rs)
	}
	awaitStatus(t, s, func(st Status) bool { return st.Name == "valid" && strings.Contains(st.LogTail, "new") })
	for _, st := range s.Status() {
		if st.Name == "ide" {
			t.Fatal("omitted prior worker retained")
		}
	}
	next.Version = "2.0.0"
	before := s.Status()[1].PID
	applyOK(t, s, next)
	if got := s.Status(); len(got) != 1 || got[0].Version != "2.0.0" || got[0].PID == before {
		t.Fatalf("replacement = %+v", got)
	}
}
func TestCachedArtifactIsVerifiedAndCannotBeSymlink(t *testing.T) {
	for _, symlink := range []bool{false, true} {
		t.Run(fmt.Sprint(symlink), func(t *testing.T) {
			s := testSupervisor(t)
			d, count := artifact(t, s, "#!/bin/sh\nexec sleep 60\n")
			applyOK(t, s, d)
			applyOK(t, s)
			path := filepath.Join(s.cacheDir, d.SHA256)
			info, err := os.Stat(path)
			if err != nil || info.Mode().Perm() != 0555 {
				t.Fatalf("cache permissions: %v %v", info, err)
			}
			if err := os.Remove(path); err != nil {
				t.Fatal(err)
			}
			if symlink {
				target := filepath.Join(t.TempDir(), "target")
				if err := os.WriteFile(target, []byte("#!/bin/sh\nexec sleep 60\n"), 0555); err != nil {
					t.Fatal(err)
				}
				if err := os.Symlink(target, path); err != nil {
					t.Fatal(err)
				}
			} else {
				if err := os.WriteFile(path, []byte("corrupt"), 0555); err != nil {
					t.Fatal(err)
				}
			}
			rs := s.Apply(context.Background(), []Desired{d})
			if len(rs) != 1 || rs[0].Error == "" || s.Status()[0].PID != 0 {
				t.Fatalf("unsafe cache executed: %+v", rs)
			}
			if count.Load() != 1 {
				t.Fatal("invalid cache unexpectedly trusted or overwritten")
			}
		})
	}
}
func TestDownloadTransportAndCancellation(t *testing.T) {
	s := testSupervisor(t)
	d, _ := artifact(t, s, "#!/bin/sh\nexec sleep 60\n")
	var hits atomic.Int32
	plain := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { hits.Add(1) }))
	defer plain.Close()
	d.URL = plain.URL
	if rs := s.Apply(context.Background(), []Desired{d}); rs[0].Error == "" {
		t.Fatal("HTTP accepted")
	}
	redirect := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { http.Redirect(w, r, plain.URL, http.StatusFound) }))
	defer redirect.Close()
	s.client = redirect.Client()
	d.URL = redirect.URL
	if rs := s.Apply(context.Background(), []Desired{d}); rs[0].Error == "" {
		t.Fatal("HTTPS downgrade accepted")
	}
	if hits.Load() != 0 {
		t.Fatal("insecure server contacted")
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if rs := s.Apply(ctx, []Desired{d}); rs[0].Error == "" {
		t.Fatal("cancelled apply started a worker")
	}
}

func TestCloseCancelsInflightDownload(t *testing.T) {
	s := testSupervisor(t)
	entered := make(chan struct{})
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { close(entered); <-r.Context().Done() }))
	defer server.Close()
	s.client = server.Client()
	d := Desired{Name: "ide", Version: "1", URL: server.URL, SHA256: strings.Repeat("a", 64)}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	applied := make(chan []Result, 1)
	go func() { applied <- s.Apply(ctx, []Desired{d}) }()
	<-entered
	closed := make(chan struct{})
	go func() { s.Close(); close(closed) }()
	select {
	case <-closed:
	case <-time.After(300 * time.Millisecond):
		cancel()
		<-closed
		t.Fatal("Close did not cancel the in-flight artifact download")
	}
	if results := <-applied; len(results) != 1 || results[0].Error == "" {
		t.Fatalf("interrupted apply=%+v", results)
	}
}
func TestDuplicateDesiredCannotStartOnRuntimeChange(t *testing.T) {
	s := testSupervisor(t)
	d, count := artifact(t, s, "#!/bin/sh\nexec sleep 60\n")
	rs := s.Apply(context.Background(), []Desired{d, d})
	if len(rs) != 2 || rs[0].Error == "" || rs[1].Error == "" {
		t.Fatalf("duplicates=%+v", rs)
	}
	if err := s.SetRuntime(context.Background(), Runtime{Node: "assigned", Namespace: "proj", URL: "ws://hub.iii.internal:50000"}); err != nil {
		t.Fatal(err)
	}
	if count.Load() != 0 || s.Status()[0].PID != 0 {
		t.Fatal("runtime change started an invalid duplicate desired worker")
	}
	applyOK(t, s, d)
	if s.Status()[0].PID == 0 {
		t.Fatal("corrected desired state did not recover")
	}
}

func TestCloseKillsStubbornDescendants(t *testing.T) {
	s := testSupervisor(t)
	marker := filepath.Join(t.TempDir(), "heartbeat")
	script := "#!/bin/sh\ntrap 'exit 0' TERM\nsh -c 'trap \"\" TERM; while :; do echo tick >> \"" + marker + "\"; sleep 0.05; done' &\necho ready\nwait\n"
	d, _ := artifact(t, s, script)
	applyOK(t, s, d)
	awaitStatus(t, s, func(st Status) bool {
		info, err := os.Stat(marker)
		return strings.Contains(st.LogTail, "ready") && err == nil && info.Size() > 0
	})
	start := time.Now()
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	if elapsed := time.Since(start); elapsed > 4*time.Second {
		t.Fatalf("stubborn shutdown took %s", elapsed)
	}
	first, err := os.Stat(marker)
	if err != nil {
		t.Fatal(err)
	}
	time.Sleep(200 * time.Millisecond)
	last, err := os.Stat(marker)
	if err != nil {
		t.Fatal(err)
	}
	if first.Size() != last.Size() {
		t.Fatal("descendant continued running after Close")
	}
}
func TestConcurrentStatusWithReconciliation(t *testing.T) {
	s := testSupervisor(t)
	d, _ := artifact(t, s, "#!/bin/sh\necho running\nexec sleep 60\n")
	applyOK(t, s, d)
	stop := make(chan struct{})
	done := make(chan struct{})
	go func() {
		defer close(done)
		for {
			select {
			case <-stop:
				return
			default:
				s.Status()
			}
		}
	}()
	defer func() { close(stop); <-done }()
	for i := range 4 {
		rt := Runtime{Node: "laptop", Namespace: "proj", URL: fmt.Sprintf("ws://hub.iii.internal:%d", 50000+i)}
		if err := s.SetRuntime(context.Background(), rt); err != nil {
			t.Fatal(err)
		}
		applyOK(t, s)
		applyOK(t, s, d)
	}
}
func TestStartFailureAndUnsafeNameRemainPerWorker(t *testing.T) {
	s := testSupervisor(t)
	broken, _ := artifact(t, s, "not an executable")
	badName := broken
	badName.Name = "../escape"
	rs := s.Apply(context.Background(), []Desired{broken, badName})
	if len(rs) != 2 || !strings.Contains(rs[0].Error, "start ide") || !strings.Contains(rs[1].Error, "safe name") {
		t.Fatalf("failures=%+v", rs)
	}
	for _, st := range s.Status() {
		if st.State != "error" || st.PID != 0 {
			t.Fatalf("status=%+v", st)
		}
	}
	good, _ := artifact(t, s, "#!/bin/sh\necho recovered\nexec sleep 60\n")
	applyOK(t, s, good)
	awaitStatus(t, s, func(st Status) bool { return strings.Contains(st.LogTail, "recovered") })
}

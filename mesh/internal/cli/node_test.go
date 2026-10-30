package cli

import (
	"context"
	"errors"
	"io"
	"net/netip"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
	"github.com/iii-hq/workers/mesh/internal/service"
)

type fakeNetwork struct {
	unnamed bool
	events  *[]string
	removed chan struct{}
	report  func() string
}

func (f *fakeNetwork) Self() netip.Addr { return netip.MustParseAddr("fd42::2") }
func (f *fakeNetwork) SelfName() (string, bool) {
	if f.unnamed {
		return "", false
	}
	return "laptop-2", true
}
func (f *fakeNetwork) Peers() []meshnet.PeerStatus {
	return []meshnet.PeerStatus{{Name: "hub", Path: "relay hub"}}
}
func (f *fakeNetwork) GatewayPort() (uint16, bool)     { return 0, false }
func (f *fakeNetwork) GatewayChanges() <-chan struct{} { return nil }
func (f *fakeNetwork) Removed() <-chan struct{}        { return f.removed }
func (f *fakeNetwork) Report(fn func() string)         { f.report = fn }
func (f *fakeNetwork) Logout() error                   { *f.events = append(*f.events, "logout"); return nil }
func (f *fakeNetwork) Close() error                    { *f.events = append(*f.events, "network close"); return nil }

type fakeWorkers struct{ events *[]string }

func (f *fakeWorkers) SetRuntime(context.Context, node.Runtime) error { return nil }
func (f *fakeWorkers) Apply(context.Context, []node.Worker) ([]node.WorkerResult, error) {
	return nil, nil
}
func (f *fakeWorkers) Status() []node.WorkerStatus { return nil }
func (f *fakeWorkers) Close() error                { *f.events = append(*f.events, "workers close"); return nil }

type fixedRunner struct{ err error }

func (f fixedRunner) Run(context.Context) error { return f.err }
func testApp(t *testing.T) (*app, *fakeNetwork, *[]string) {
	t.Helper()
	dir := t.TempDir()
	events := []string{}
	network := &fakeNetwork{events: &events, removed: make(chan struct{})}
	a := &app{serveStatus: func(report func() string) (io.Closer, error) {
		if !strings.Contains(report(), "removed from the mesh") {
			t.Fatal(report())
		}
		return io.NopCloser(strings.NewReader("")), nil
	}, paths: service.Paths{State: filepath.Join(dir, "state"), Cache: filepath.Join(dir, "cache"), Bin: filepath.Join(dir, "mesh")}, version: "0.1.0", up: func(State, string) (networkService, error) {
		events = append(events, "network up")
		return network, nil
	}, workers: func(State) (workerService, error) { return &fakeWorkers{&events}, nil }, newNode: func(o node.Options) (runner, error) { return node.New(o) }}
	if err := saveState(a.paths.State, testState(t)); err != nil {
		t.Fatal(err)
	}
	return a, network, &events
}
func TestNodeCancellationPersistsAssignedNameAndCleans(t *testing.T) {
	a, _, events := testApp(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := a.node(ctx); err != nil {
		t.Fatal(err)
	}
	state, err := loadState(a.paths.State)
	if err != nil {
		t.Fatal(err)
	}
	if state.Name != "laptop-2" {
		t.Fatal(state.Name)
	}
	if !reflect.DeepEqual(*events, []string{"network up", "workers close", "network close"}) {
		t.Fatal(*events)
	}
}
func TestNodeReturnOwnership(t *testing.T) {
	for _, returned := range []error{node.ErrRestartRequired, errors.New("runtime failed"), node.ErrRemoved} {
		t.Run(returned.Error(), func(t *testing.T) {
			a, _, events := testApp(t)
			a.newNode = func(node.Options) (runner, error) { return fixedRunner{returned}, nil }
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			done := make(chan error, 1)
			go func() { done <- a.node(ctx) }()
			if errors.Is(returned, node.ErrRemoved) {
				deadline := time.After(time.Second)
				for !exists(filepath.Join(a.paths.State, "removed")) {
					select {
					case <-deadline:
						t.Fatal("missing removed marker")
					default:
						time.Sleep(time.Millisecond)
					}
				}
				select {
				case err := <-done:
					t.Fatalf("removed service exited %v", err)
				default:
				}
				cancel()
			}
			err := <-done
			if !errors.Is(returned, node.ErrRemoved) && !errors.Is(err, returned) {
				t.Fatalf("%v", err)
			}
			if !reflect.DeepEqual(*events, []string{"network up", "workers close", "network close"}) {
				t.Fatal(*events)
			}
			if errors.Is(returned, node.ErrRemoved) {
				*events = nil
				ctx, cancel := context.WithCancel(context.Background())
				cancel()
				if err := a.node(ctx); err != nil {
					t.Fatal(err)
				}
				if len(*events) != 0 {
					t.Fatal("removed node restarted", *events)
				}
			}
		})
	}
}
func TestLeaveLogsOutBeforeCleanup(t *testing.T) {
	a, _, events := testApp(t)
	a.uninstall = func() error {
		*events = append(*events, "stop service")
		ctx, cancel := context.WithCancel(context.Background())
		cancel()
		return a.node(ctx)
	}
	a.clean = func() error { *events = append(*events, "native clean"); return nil }
	os.MkdirAll(a.paths.Cache, 0755)
	os.WriteFile(filepath.Join(a.paths.Cache, "artifact"), []byte("cache"), 0555)
	if err := a.leave(context.Background()); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(*events, []string{"stop service", "network up", "logout", "network close", "native clean"}) {
		t.Fatal(*events)
	}
	for _, path := range []string{a.paths.State, a.paths.Cache} {
		if exists(path) {
			t.Fatal("left state", path)
		}
	}
}
func TestLeaveWithoutLogoutAcknowledgementRetainsCredentials(t *testing.T) {
	a, _, _ := testApp(t)
	a.uninstall = func() error { return nil }
	a.clean = func() error { t.Fatal("cleaned before logout acknowledged"); return nil }
	if err := a.leave(context.Background()); err == nil {
		t.Fatal("pretended to log out")
	}
	if _, err := loadState(a.paths.State); err != nil {
		t.Fatal(err)
	}
}

func TestBackendRemovalMarkerPreventsRestart(t *testing.T) {
	a, _, events := testApp(t)
	os.MkdirAll(filepath.Join(a.paths.State, "mesh"), 0700)
	os.WriteFile(filepath.Join(a.paths.State, "mesh", "removed"), nil, 0600)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := a.node(ctx); err != nil {
		t.Fatal(err)
	}
	if len(*events) != 0 {
		t.Fatal("restarted revoked backend", *events)
	}
}
func TestNodeConstructorFailureClosesOwnedResources(t *testing.T) {
	a, _, events := testApp(t)
	failure := errors.New("construction failed")
	a.newNode = func(node.Options) (runner, error) { return nil, failure }
	if err := a.node(context.Background()); !errors.Is(err, failure) {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(*events, []string{"network up", "workers close", "network close"}) {
		t.Fatal(*events)
	}
}

func TestLeaveStopsActiveNodeAfterWorkersAndLogout(t *testing.T) {
	a, _, events := testApp(t)
	ready := make(chan struct{})
	a.newNode = func(o node.Options) (runner, error) { n, err := node.New(o); close(ready); return n, err }
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() { done <- a.node(ctx) }()
	<-ready
	a.uninstall = func() error { *events = append(*events, "stop service"); cancel(); return <-done }
	a.clean = func() error { *events = append(*events, "native clean"); return nil }
	if err := a.leave(context.Background()); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(*events, []string{"network up", "stop service", "workers close", "logout", "network close", "native clean"}) {
		t.Fatal(*events)
	}
}

func TestPendingLeaveRequiresAuthenticatedEnrollment(t *testing.T) {
	a, network, events := testApp(t)
	network.unnamed = true
	if err := writePrivate(a.paths.State, "leave-request", []byte("logout")); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := a.node(ctx); !errors.Is(err, context.Canceled) {
		t.Fatalf("acknowledged logout before enrollment: %v", err)
	}
	if !reflect.DeepEqual(*events, []string{"network up", "network close"}) {
		t.Fatal(*events)
	}
	if exists(filepath.Join(a.paths.State, "logout-result")) {
		t.Fatal("logout acknowledgement without authenticated map")
	}
}

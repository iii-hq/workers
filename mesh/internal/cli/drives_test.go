package cli

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/iii-hq/workers/mesh/internal/node"
)

type fakeDrives struct {
	events  *[]string
	failure error
}

func (d *fakeDrives) Apply(context.Context, node.DrivesRequest) ([]node.DriveStatus, error) {
	return nil, nil
}
func (d *fakeDrives) Status() []node.DriveStatus { return nil }
func (d *fakeDrives) Close() error {
	if d.events != nil {
		*d.events = append(*d.events, "drives close")
	}
	return nil
}
func (d *fakeDrives) Drop(context.Context) error {
	if d.events != nil {
		*d.events = append(*d.events, "drives drop")
	}
	return d.failure
}
func TestDriveServiceRestartPreservesAndRemovalDrops(t *testing.T) {
	for _, removed := range []bool{false, true} {
		t.Run(map[bool]string{false: "restart", true: "removal"}[removed], func(t *testing.T) {
			a, _, events := testApp(t)
			a.drives = func(State) (driveService, error) { return &fakeDrives{events: events}, nil }
			a.newNode = func(o node.Options) (runner, error) {
				if o.Drives == nil {
					t.Fatal("missing production drive controller")
				}
				if removed {
					return fixedRunner{node.ErrRemoved}, nil
				}
				return fixedRunner{node.ErrRestartRequired}, nil
			}
			ctx, cancel := context.WithCancel(context.Background())
			cancel()
			err := a.node(ctx)
			if !removed && !errors.Is(err, node.ErrRestartRequired) {
				t.Fatal(err)
			}
			want := []string{"network up", "workers close", "drives close", "network close"}
			if removed {
				want = []string{"network up", "workers close", "drives drop", "drives close", "network close"}
			}
			if !reflect.DeepEqual(*events, want) {
				t.Fatalf("%v want %v", *events, want)
			}
		})
	}
}
func TestDriveRemovalMarkerRecoveryAndFailedLeaveRetainsState(t *testing.T) {
	a, _, events := testApp(t)
	failure := errors.New("operator session unavailable")
	a.drives = func(State) (driveService, error) { return &fakeDrives{events: events, failure: failure}, nil }
	os.WriteFile(filepath.Join(a.paths.State, "removed"), nil, 0600)
	os.WriteFile(filepath.Join(a.paths.State, "leave-request"), nil, 0600)
	if err := a.node(context.Background()); !errors.Is(err, failure) {
		t.Fatal("marker acknowledged without cleanup", err)
	}
	b, _ := os.ReadFile(filepath.Join(a.paths.State, "logout-result"))
	if string(b) == "ok" {
		t.Fatal("cleanup failure acknowledged")
	}
	a.uninstall = func() error { return nil }
	a.clean = func() error { t.Fatal("teardown before drives cleanup"); return nil }
	if err := a.leave(context.Background()); !errors.Is(err, failure) {
		t.Fatal(err)
	}
	if _, err := loadState(a.paths.State); err != nil {
		t.Fatal("lost retry state", err)
	}
}
func TestPendingLeaveDropsDrivesBeforeLogout(t *testing.T) {
	a, _, events := testApp(t)
	a.drives = func(State) (driveService, error) { return &fakeDrives{events: events}, nil }
	os.WriteFile(filepath.Join(a.paths.State, "leave-request"), nil, 0600)
	if err := a.node(context.Background()); err != nil {
		t.Fatal(err)
	}
	want := []string{"network up", "drives drop", "logout", "drives close", "network close"}
	if !reflect.DeepEqual(*events, want) {
		t.Fatalf("%v want %v", *events, want)
	}
}

package node

import (
	"context"
	"sync"

	"github.com/iii-hq/workers/mesh/internal/drives"
)

type managedDrives struct{ manager *drives.Manager }

func NewDriveController(m *drives.Manager) DriveController { return &managedDrives{m} }
func (d *managedDrives) Apply(ctx context.Context, w DrivesRequest) ([]DriveStatus, error) {
	return d.manager.Apply(ctx, w), nil
}
func (d *managedDrives) Status() []DriveStatus { return d.manager.Status() }

// UnsupportedDrives reports desired drives as "unsupported" without an Error,
// so the hub's reconcile succeeds instead of retrying every second.
type UnsupportedDrives struct {
	mu   sync.Mutex
	last []DriveStatus
}

func (d *UnsupportedDrives) Apply(_ context.Context, r DrivesRequest) ([]DriveStatus, error) {
	results := []DriveStatus{}
	for _, e := range r.Serve {
		results = append(results, DriveStatus{Name: e.Name, Serve: true, State: "unsupported"})
	}
	for _, m := range r.Mount {
		results = append(results, DriveStatus{Name: m.Name, State: "unsupported"})
	}
	d.mu.Lock()
	d.last = results
	d.mu.Unlock()
	return results, nil
}
func (d *UnsupportedDrives) Status() []DriveStatus {
	d.mu.Lock()
	defer d.mu.Unlock()
	return append([]DriveStatus{}, d.last...)
}

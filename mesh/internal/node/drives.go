package node

import (
	"context"
	"github.com/iii-hq/workers/mesh/internal/drives"
)

type managedDrives struct{ manager *drives.Manager }

func NewDriveController(m *drives.Manager) DriveController { return &managedDrives{m} }
func (d *managedDrives) Apply(ctx context.Context, w DrivesRequest) ([]DriveStatus, error) {
	return d.manager.Apply(ctx, w), nil
}
func (d *managedDrives) Status() []DriveStatus { return d.manager.Status() }

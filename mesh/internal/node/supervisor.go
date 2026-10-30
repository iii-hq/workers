package node

import (
	"context"

	"github.com/iii-hq/workers/mesh/internal/supervisor"
)

type supervisorController struct{ supervisor *supervisor.Supervisor }

// NewWorkerController borrows the supervisor. Its owner must Close it after
// Node.Run returns, including removal and update-triggered service restarts.
func NewWorkerController(s *supervisor.Supervisor) WorkerController {
	return &supervisorController{supervisor: s}
}
func (c *supervisorController) SetRuntime(ctx context.Context, rt Runtime) error {
	return c.supervisor.SetRuntime(ctx, supervisor.Runtime{Node: rt.Node, Namespace: rt.Namespace, URL: rt.URL})
}
func (c *supervisorController) Apply(ctx context.Context, workers []Worker) ([]WorkerResult, error) {
	desired := make([]supervisor.Desired, 0, len(workers))
	for _, w := range workers {
		desired = append(desired, supervisor.Desired{Name: w.Name, Version: w.Version, URL: w.ArtifactURL, SHA256: w.SHA256})
	}
	applied := c.supervisor.Apply(ctx, desired)
	results := make([]WorkerResult, 0, len(applied))
	for _, r := range applied {
		results = append(results, WorkerResult{Name: r.Name, Error: r.Error})
	}
	return results, nil
}
func (c *supervisorController) Status() []WorkerStatus {
	statuses := c.supervisor.Status()
	result := make([]WorkerStatus, 0, len(statuses))
	for _, s := range statuses {
		result = append(result, WorkerStatus{Name: s.Name, Version: s.Version, State: s.State, Restarts: s.Restarts, LogTail: s.LogTail, Error: s.Error})
	}
	return result
}

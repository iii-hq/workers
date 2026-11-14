package hub

import (
	"sync"
	"time"
)

// retryMin is the first wait of a retry; a variable so tests can lower it.
var retryMin = time.Second

// retry runs a failed pass again after a doubling wait. An event that may have ended the failure calls reset and runs the pass at once.
type retry struct {
	mu    sync.Mutex
	timer *time.Timer
	wait  time.Duration
}

func (r *retry) reset() {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.timer != nil {
		r.timer.Stop()
	}
	r.wait = 0
}

func (r *retry) again(run func()) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.timer != nil {
		r.timer.Stop()
	}
	r.wait = min(max(retryMin, 2*r.wait), time.Minute)
	r.timer = time.AfterFunc(r.wait, run)
}

// transient marks a failure that no event ends, the only kind that is retried.
type transient struct{ error }

func (t transient) Unwrap() error { return t.error }

// callFailed marks a failed call for retry unless the function is missing: its arrival is an engine event.
func callFailed(err error) error {
	if err == nil || remoteCode(err, "function_not_found") {
		return err
	}
	return transient{err}
}

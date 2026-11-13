package hub

import (
	"context"
	"errors"
	"log"
	"sync"
	"time"

	"github.com/iii-hq/workers/mesh/internal/coordinator"
)

// nodeRefreshDelay folds a burst of engine function changes into one re-read of the online nodes.
var nodeRefreshDelay = 250 * time.Millisecond

type nodeReconcileTask struct {
	cancel context.CancelFunc
	dirty  bool
	wake   chan struct{} // an event of this node cuts the wait of a failed read
}
type nodeReconciler struct {
	hub    *Hub
	ctx    context.Context
	cancel context.CancelFunc
	mu     sync.Mutex
	tasks  map[string]*nodeReconcileTask
	wg     sync.WaitGroup
	done   chan struct{}
}

func (h *Hub) startNodeCalls(ctx context.Context) {
	ctx, cancel := context.WithCancel(ctx)
	r := &nodeReconciler{hub: h, ctx: ctx, cancel: cancel, tasks: map[string]*nodeReconcileTask{}, done: make(chan struct{})}
	h.mu.Lock()
	h.nodeReconciler = r
	h.mu.Unlock()
	// Subscribed before the first read: the coordinator replays nothing.
	events := h.events
	h.Coordinator.SetNodeObserver(func(change coordinator.NodeChange) {
		if events != nil {
			events.publish(change)
		}
		switch change.Event {
		case "online":
			r.enqueue(change.Name)
		case "offline", "removed":
			r.remove(change.Name)
		}
	})
	go h.refreshPass() // the first node calls carry the catalog
	go func() {
		defer close(r.done)
		<-ctx.Done()
		r.mu.Lock() // an enqueue that saw the live context has added its task
		r.mu.Unlock()
		r.wg.Wait()
	}()
}

// onFunctionsChange re-reads the catalog and the online nodes: a bridge shares or drops workers over its open connection, with no new gateway auth.
func (h *Hub) onFunctionsChange() {
	if h.refreshing.CompareAndSwap(false, true) {
		time.AfterFunc(nodeRefreshDelay, h.refreshPass)
	}
}

// refreshPass reads the catalog, then queues the online nodes, which receive it.
// A failed read keeps the last catalog and is tried again after a backoff.
func (h *Hub) refreshPass() {
	h.passMu.Lock()
	defer h.passMu.Unlock()
	h.refreshing.Store(false) // before the read: a change during it schedules another pass
	if h.passRetry != nil {
		h.passRetry.Stop() // this pass replaces a pending retry, so an outage keeps one retry chain
	}
	h.mu.RLock()
	r := h.nodeReconciler
	h.mu.RUnlock()
	if r == nil || r.ctx.Err() != nil {
		return
	}
	ctx, cancel := context.WithTimeout(r.ctx, 15*time.Second)
	err := h.refreshShared(ctx)
	cancel()
	h.queueNodes() // node details do not depend on the catalog
	if err == nil {
		h.passBackoff = 0
		return
	}
	h.passBackoff = min(max(time.Second, 2*h.passBackoff), time.Minute)
	log.Printf("mesh share: read shared functions: %v (retry in %s)", err, h.passBackoff)
	h.passRetry = time.AfterFunc(h.passBackoff, h.onFunctionsChange)
}
func (h *Hub) queueNode(name string) {
	h.mu.RLock()
	r := h.nodeReconciler
	h.mu.RUnlock()
	if r != nil {
		r.enqueue(name)
	}
}
func (h *Hub) queueNodes() {
	if h.Coordinator == nil {
		return
	}
	for _, n := range h.Coordinator.Nodes() {
		if n.Name != "hub" && n.Online {
			h.queueNode(n.Name)
		}
	}
}
func (h *Hub) cancelNode(name string) {
	h.mu.Lock()
	r := h.nodeReconciler
	delete(h.nodeDetails, name)
	h.mu.Unlock()
	if r != nil {
		r.remove(name)
	}
}
func (h *Hub) stopNodeCalls() {
	h.mu.RLock()
	r := h.nodeReconciler
	h.mu.RUnlock()
	if r != nil {
		r.cancel()
	}
}
func (r *nodeReconciler) enqueue(name string) {
	if name == "hub" {
		return
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.ctx.Err() != nil {
		return
	}
	if task := r.tasks[name]; task != nil {
		task.dirty = true
		select {
		case task.wake <- struct{}{}:
		default:
		}
		return
	}
	ctx, cancel := context.WithCancel(r.ctx)
	task := &nodeReconcileTask{cancel: cancel, wake: make(chan struct{}, 1)}
	r.tasks[name] = task
	r.wg.Add(1)
	go r.reconcile(ctx, name, task)
}
func (r *nodeReconciler) remove(name string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if task := r.tasks[name]; task != nil {
		task.cancel()
		delete(r.tasks, name)
	}
}
func (r *nodeReconciler) reconcile(ctx context.Context, name string, task *nodeReconcileTask) {
	defer r.wg.Done()
	defer task.cancel()
	defer func() {
		r.mu.Lock()
		if r.tasks[name] == task {
			delete(r.tasks, name)
		}
		r.mu.Unlock()
	}()
	lastError := ""
	wait := time.Duration(0)
	for ctx.Err() == nil {
		r.mu.Lock()
		if task.dirty {
			wait = 0 // an event may have ended the failure: the waits start over
		}
		task.dirty = false
		select {
		case <-task.wake: // this pass reads what that event announced
		default:
		}
		r.mu.Unlock()
		err := r.hub.OnNodeOnline(ctx, name)
		if ctx.Err() != nil {
			return
		}
		r.mu.Lock()
		dirty := task.dirty
		if err == nil && !dirty {
			if r.tasks[name] == task {
				delete(r.tasks, name)
			}
			r.mu.Unlock()
			return
		}
		r.mu.Unlock()
		if err != nil && err.Error() != lastError {
			lastError = err.Error()
			log.Printf("mesh node %s: %v", name, err)
		}
		if err == nil {
			continue
		}
		var retry <-chan time.Time // nil: only an event of this node reads it again
		// A node that answers again announces nothing; a new version or a node function registering queues the node.
		if !errors.Is(err, errOtherVersion) && !remoteCode(err, "function_not_found") {
			wait = min(max(retryMin, 2*wait), time.Minute)
			retry = time.NewTimer(wait).C
		}
		select {
		case <-ctx.Done():
			return
		case <-task.wake:
		case <-retry:
		}
	}
}

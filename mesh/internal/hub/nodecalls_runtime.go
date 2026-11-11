package hub

import (
	"context"
	"log"
	"sync"
	"time"
)

// nodeRefreshTicks is how many 1 s watch ticks pass between status re-reads of an online node.
var nodeRefreshTicks = 30

type nodeReconcileTask struct {
	cancel context.CancelFunc
	dirty  bool
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
	go r.watch()
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
		return
	}
	ctx, cancel := context.WithCancel(r.ctx)
	task := &nodeReconcileTask{cancel: cancel}
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
	for ctx.Err() == nil {
		r.mu.Lock()
		task.dirty = false
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
		timer := time.NewTimer(time.Second)
		select {
		case <-ctx.Done():
			timer.Stop()
			return
		case <-timer.C:
		}
	}
}
func (r *nodeReconciler) watch() {
	defer close(r.done)
	defer r.wg.Wait()
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	previous := map[string]bool{}
	for tick := 0; ; tick++ {
		current := map[string]bool{}
		for _, n := range r.hub.Coordinator.Nodes() {
			if n.Name == "hub" || !n.Online {
				continue
			}
			current[n.Name] = true
			// A bridge shares or drops workers over its open connection, with no new gateway auth; re-read every online node so nodes::list follows.
			if !previous[n.Name] || tick%nodeRefreshTicks == 0 {
				r.enqueue(n.Name)
			}
		}
		for name := range previous {
			if !current[name] {
				r.remove(name)
			}
		}
		previous = current
		select {
		case <-r.ctx.Done():
			return
		case <-ticker.C:
		}
	}
}

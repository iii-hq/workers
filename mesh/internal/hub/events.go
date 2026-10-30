package hub

import (
	"context"
	"encoding/json"
	"errors"
	"sync"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
)

const nodeEventCapacity = 256

type nodeEvent struct {
	Name  string    `json:"name"`
	Event string    `json:"event"`
	At    time.Time `json:"at"`
}

type nodeBinding struct {
	function, namespace, node string
	ctx                       context.Context
	cancel                    context.CancelFunc
	stop                      func() bool
}

func (b *nodeBinding) close() { b.stop(); b.cancel() }

type nodeDelivery struct {
	binding *nodeBinding
	event   nodeEvent
}

type nodeEvents struct {
	mu       sync.Mutex
	bindings map[string]*nodeBinding
	queue    chan nodeDelivery
	ctx      context.Context
	cancel   context.CancelFunc
	done     chan struct{}
	client   *iii.Client
	version  string
}

func (h *Hub) registerNodeEvents(ctx context.Context, client *iii.Client) error {
	h.reload.Lock()
	defer h.reload.Unlock()
	if h.closed || h.events != nil {
		return errors.New("node events already started or hub closed")
	}
	ctx, cancel := context.WithCancel(ctx)
	e := &nodeEvents{bindings: make(map[string]*nodeBinding), queue: make(chan nodeDelivery, nodeEventCapacity), ctx: ctx, cancel: cancel, done: make(chan struct{}), client: client, version: h.Version}
	err := client.RegisterTriggerType("mesh::node-changed", "Mesh node lifecycle changes", e, iii.RegisterTriggerTypeOptions{
		TriggerRequestFormat: json.RawMessage(`{"type":"object","properties":{"node":{"type":"string","minLength":1}},"additionalProperties":false}`),
		CallRequestFormat:    json.RawMessage(`{"type":"object","properties":{"name":{"type":"string"},"event":{"enum":["joined","online","offline","removed","outdated"]},"at":{"type":"string","format":"date-time"}},"required":["name","event","at"],"additionalProperties":false}`),
	})
	if err != nil {
		cancel()
		return err
	}
	h.events = e
	go e.run()
	if h.Coordinator != nil {
		h.Coordinator.SetNodeObserver(e.publish)
	}
	return nil
}

func (e *nodeEvents) RegisterTrigger(ctx context.Context, cfg iii.TriggerConfig) error {
	var fields map[string]json.RawMessage
	if json.Unmarshal(cfg.Config, &fields) != nil || fields == nil {
		return errors.New("node event config must be an object")
	}
	filter := ""
	for k, v := range fields {
		if k != "node" {
			return errors.New("unknown node event config key")
		}
		if json.Unmarshal(v, &filter) != nil || filter == "" || coordinator.SanitizeName(filter) != filter {
			return errors.New("invalid node event filter")
		}
	}
	if cfg.ID == "" || cfg.FunctionID == "" {
		return errors.New("node event binding requires id and function")
	}
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.ctx.Err() != nil || ctx.Err() != nil {
		return errors.New("node events closed")
	}
	for id, b := range e.bindings {
		if b.ctx.Err() != nil {
			b.close()
			delete(e.bindings, id)
		}
	}
	old := e.bindings[cfg.ID]
	if old == nil && len(e.bindings) >= nodeEventCapacity {
		return errors.New("too many node event bindings")
	}
	if old != nil {
		old.close()
	}
	bindingCtx, cancel := context.WithCancel(ctx)
	e.bindings[cfg.ID] = &nodeBinding{function: cfg.FunctionID, namespace: cfg.Namespace, node: filter, ctx: bindingCtx, cancel: cancel, stop: context.AfterFunc(e.ctx, cancel)}
	return nil
}
func (e *nodeEvents) UnregisterTrigger(_ context.Context, cfg iii.TriggerConfig) error {
	e.mu.Lock()
	defer e.mu.Unlock()
	if b := e.bindings[cfg.ID]; b != nil {
		b.close()
		delete(e.bindings, cfg.ID)
	}
	return nil
}

func (e *nodeEvents) publish(change coordinator.NodeChange) {
	if change.Event == "version" {
		if change.Version == "" || change.Version == e.version {
			return
		}
		change.Event = "outdated"
	}
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.ctx.Err() != nil {
		return
	}
	for _, b := range e.bindings {
		if b.ctx.Err() != nil || b.node != "" && b.node != change.Name {
			continue
		}
		// Capture the binding generation now so a later registration cannot see history.
		select {
		case e.queue <- nodeDelivery{b, nodeEvent{change.Name, change.Event, change.At}}:
		default:
		}
	}
}
func (e *nodeEvents) run() {
	defer close(e.done)
	for {
		select {
		case <-e.ctx.Done():
			return
		case d := <-e.queue:
			if d.binding.ctx.Err() != nil || e.client.State() != iii.StateConnected {
				continue
			}
			data, _ := json.Marshal(d.event)
			_, _ = e.client.Trigger(d.binding.ctx, iii.TriggerRequest{FunctionID: d.binding.function, Namespace: d.binding.namespace, Data: data, Timeout: 5 * time.Second})
		}
	}
}
func (e *nodeEvents) close() {
	e.cancel()
	e.mu.Lock()
	for id, b := range e.bindings {
		b.close()
		delete(e.bindings, id)
	}
	e.mu.Unlock()
	<-e.done
}

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
	node     string
	observer *iii.Observer
	ctx      context.Context
	cancel   context.CancelFunc
}

func (b *nodeBinding) close() { b.cancel() }

type nodeDelivery struct {
	binding *nodeBinding
	event   nodeEvent
}

type nodeEvents struct {
	mu          sync.Mutex
	bindings    map[string]*nodeBinding
	queue       chan nodeDelivery
	ctx         context.Context
	cancel      context.CancelFunc
	done        chan struct{}
	client      *iii.Client
	version     string
	unsubscribe func()
}

func (h *Hub) registerNodeEvents(ctx context.Context, client *iii.Client) error {
	h.reload.Lock()
	defer h.reload.Unlock()
	if h.closed || h.events != nil {
		return errors.New("node events already started or hub closed")
	}
	ctx, cancel := context.WithCancel(ctx)
	e := &nodeEvents{bindings: make(map[string]*nodeBinding), queue: make(chan nodeDelivery, nodeEventCapacity), ctx: ctx, cancel: cancel, done: make(chan struct{}), client: client, version: h.Version}
	client.OnObserverAddedFor("mesh::on-node-changed", e.addObserver)
	client.OnObserverRemovedFor("mesh::on-node-changed", func(_ context.Context, obs *iii.Observer) { e.removeObserver(obs) })
	client.OnObservationMatchFor("mesh::on-node-changed", iii.MatchFunc(func(obs *iii.Observer, call *iii.Call) bool {
		var meta struct {
			ForObserver string `json:"for_observer"`
		}
		_ = json.Unmarshal(call.Metadata, &meta)
		return meta.ForObserver == obs.ID
	}))
	client.OnObservationDeliverFor("mesh::on-node-changed", func(ctx context.Context, call *iii.Call, observers []*iii.Observer) (any, error) {
		for _, obs := range observers {
			if obs.Matches(call) {
				_, _ = client.Call(ctx, iii.CallRequest{Observer: obs, Payload: call.Input, Timeout: 5 * time.Second})
			}
		}
		return json.RawMessage(call.Input), nil
	})
	_, err := iii.RegisterFunctionTyped(client, "mesh::on-node-changed", func(_ context.Context, event nodeEvent) (nodeEvent, error) { return event, nil }, iii.RegisterFunctionOptions{
		Private: true, Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`),
		ArgsFormat: json.RawMessage(`{"type":"object","properties":{"node":{"type":"string","minLength":1}},"additionalProperties":false}`),
	})
	if err != nil {
		cancel()
		return err
	}
	e.unsubscribe = client.AddConnectionStateListener(func(state iii.ConnectionState) {
		e.mu.Lock()
		defer e.mu.Unlock()
		for id, b := range e.bindings {
			if state != iii.StateConnected {
				b.close()
				continue
			}
			if b.ctx.Err() != nil && e.ctx.Err() == nil {
				// A resumed observer starts a new generation. Queued deliveries still
				// hold the canceled generation and can never turn into replayed history.
				bindingCtx, cancel := context.WithCancel(e.ctx)
				e.bindings[id] = &nodeBinding{node: b.node, observer: b.observer, ctx: bindingCtx, cancel: cancel}
			}
		}
	})
	h.events = e
	go e.run()
	if h.Coordinator != nil {
		h.Coordinator.SetNodeObserver(e.publish)
	}
	return nil
}

func (e *nodeEvents) addObserver(_ context.Context, obs *iii.Observer) error {
	var fields map[string]json.RawMessage
	if obs.DecodeArgs(&fields) != nil || fields == nil {
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
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.ctx.Err() != nil {
		return errors.New("node events closed")
	}
	for id, b := range e.bindings {
		if b.ctx.Err() != nil {
			b.close()
			delete(e.bindings, id)
		}
	}
	old := e.bindings[obs.ID]
	if old == nil && len(e.bindings) >= nodeEventCapacity {
		return errors.New("too many node event bindings")
	}
	if old != nil {
		old.close()
	}
	bindingCtx, cancel := context.WithCancel(e.ctx)
	e.bindings[obs.ID] = &nodeBinding{observer: obs, node: filter, ctx: bindingCtx, cancel: cancel}
	return nil
}
func (e *nodeEvents) removeObserver(obs *iii.Observer) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if b := e.bindings[obs.ID]; b != nil && b.observer == obs {
		b.close()
		delete(e.bindings, obs.ID)
	}
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
	if e.ctx.Err() != nil || e.client.ConnectionState() != iii.StateConnected {
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
			if d.binding.ctx.Err() != nil || e.client.ConnectionState() != iii.StateConnected {
				continue
			}
			if d.binding.observer.Accepted(d.binding.ctx) != nil {
				continue
			}
			data, _ := json.Marshal(d.event)
			meta, _ := json.Marshal(map[string]string{"for_observer": d.binding.observer.ID})
			_, _ = e.client.Call(d.binding.ctx, iii.CallRequest{FunctionID: "mesh::on-node-changed", Payload: data, Metadata: meta, Timeout: 5 * time.Second})
		}
	}
}
func (e *nodeEvents) close() {
	e.unsubscribe()
	e.cancel()
	e.mu.Lock()
	for id, b := range e.bindings {
		b.close()
		delete(e.bindings, id)
	}
	e.mu.Unlock()
	<-e.done
}

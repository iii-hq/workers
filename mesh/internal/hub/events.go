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
	generation uint64
	delivery   sync.Mutex
	node       string
	observer   *iii.Observer
	ctx        context.Context
	cancel     context.CancelFunc
}

func (b *nodeBinding) close() { b.cancel() }

// Draining after cancellation orders any admitted activation before a later
// observer ack can reuse its ID. Never drain while holding nodeEvents.mu.
func (b *nodeBinding) drain() { b.delivery.Lock(); b.delivery.Unlock() }

type nodeEventMetadata struct {
	ForObserver string `json:"for_observer"`
	Generation  uint64 `json:"mesh_binding_generation"`
}

type nodeDelivery struct {
	binding *nodeBinding
	event   nodeEvent
}

type nodeEvents struct {
	mu             sync.Mutex
	bindings       map[string]*nodeBinding
	queue          chan nodeDelivery
	ctx            context.Context
	cancel         context.CancelFunc
	done           chan struct{}
	client         *iii.Client
	version        string
	unsubscribe    func()
	nextGeneration uint64
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
	client.OnObservationDeliverFor("mesh::on-node-changed", e.deliver)
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
		var retired []*nodeBinding
		for id, b := range e.bindings {
			if state != iii.StateConnected {
				b.close()
				retired = append(retired, b)
				continue
			}
			if b.ctx.Err() != nil && e.ctx.Err() == nil {
				// A resumed observer starts a new generation. Queued deliveries still
				// hold the canceled generation and can never turn into replayed history.
				e.bindings[id] = e.newBinding(b.observer, b.node)
			}
		}
		e.mu.Unlock()
		for _, b := range retired {
			b.drain()
		}
	})
	h.events = e
	go e.run()
	if h.Coordinator != nil {
		h.Coordinator.SetNodeObserver(e.publish)
	}
	return nil
}

// The caller holds e.mu. A resumed SDK Observer can retain its object identity,
// so each binding lifetime needs an independent generation.
func (e *nodeEvents) newBinding(obs *iii.Observer, node string) *nodeBinding {
	e.nextGeneration++
	ctx, cancel := context.WithCancel(e.ctx)
	return &nodeBinding{generation: e.nextGeneration, observer: obs, node: node, ctx: ctx, cancel: cancel}
}

func (e *nodeEvents) deliver(ctx context.Context, call *iii.Call, observers []*iii.Observer) (any, error) {
	var meta nodeEventMetadata
	if json.Unmarshal(call.Metadata, &meta) != nil {
		return json.RawMessage(call.Input), nil
	}
	for _, obs := range observers {
		if !obs.Matches(call) {
			continue
		}
		e.mu.Lock()
		binding := e.bindings[obs.ID]
		e.mu.Unlock()
		if binding == nil {
			continue
		}
		binding.delivery.Lock()
		e.mu.Lock()
		valid := e.bindings[obs.ID] == binding && binding.observer == obs && meta.ForObserver == obs.ID && meta.Generation == binding.generation && binding.ctx.Err() == nil && e.ctx.Err() == nil && ctx.Err() == nil && e.client.ConnectionState() == iii.StateConnected
		var callCtx context.Context
		var cancel context.CancelFunc
		var stop func() bool
		if valid {
			callCtx, cancel = context.WithCancel(ctx)
			stop = context.AfterFunc(binding.ctx, cancel)
		}
		e.mu.Unlock()
		if valid {
			_, _ = e.client.Call(callCtx, iii.CallRequest{Observer: obs, Payload: call.Input, Timeout: 5 * time.Second})
			stop()
			cancel()
		}
		binding.delivery.Unlock()
	}
	return json.RawMessage(call.Input), nil
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
	var retired []*nodeBinding
	defer func() {
		e.mu.Unlock()
		for _, b := range retired {
			b.drain()
		}
	}()
	if e.ctx.Err() != nil {
		return errors.New("node events closed")
	}
	for id, b := range e.bindings {
		if b.ctx.Err() != nil {
			b.close()
			retired = append(retired, b)
			delete(e.bindings, id)
		}
	}
	old := e.bindings[obs.ID]
	if old == nil && len(e.bindings) >= nodeEventCapacity {
		return errors.New("too many node event bindings")
	}
	if old != nil {
		old.close()
		retired = append(retired, old)
	}
	e.bindings[obs.ID] = e.newBinding(obs, filter)
	return nil
}
func (e *nodeEvents) removeObserver(obs *iii.Observer) {
	e.mu.Lock()
	b := e.bindings[obs.ID]
	if b != nil && b.observer == obs {
		b.close()
		delete(e.bindings, obs.ID)
	} else {
		b = nil
	}
	e.mu.Unlock()
	if b != nil {
		b.drain()
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
			meta, _ := json.Marshal(nodeEventMetadata{ForObserver: d.binding.observer.ID, Generation: d.binding.generation})
			_, _ = e.client.Call(d.binding.ctx, iii.CallRequest{FunctionID: "mesh::on-node-changed", Payload: data, Metadata: meta, Timeout: 5 * time.Second})
		}
	}
}
func (e *nodeEvents) close() {
	e.unsubscribe()
	e.cancel()
	e.mu.Lock()
	var retired []*nodeBinding
	for id, b := range e.bindings {
		b.close()
		retired = append(retired, b)
		delete(e.bindings, id)
	}
	e.mu.Unlock()
	for _, b := range retired {
		b.drain()
	}
	<-e.done
}

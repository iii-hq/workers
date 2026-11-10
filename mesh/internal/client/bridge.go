package client

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"maps"
	"net/http"
	"slices"
	"strings"
	"sync"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/node"
)

type caller interface {
	Call(context.Context, iii.CallRequest) (json.RawMessage, error)
}

type function struct {
	worker string
	opts   iii.RegisterFunctionOptions
}

// SharedStatus is one share entry in the local mesh::status.
type SharedStatus struct {
	Worker    string   `json:"worker"`
	State     string   `json:"state"`
	Functions []string `json:"functions"`
}

// Bridge is the client node's WorkerController: its workers are the shared
// local workers, whose functions it registers on one hub connection and
// forwards to the local engine.
type Bridge struct {
	local     caller
	namespace string // local project namespace

	mu        sync.Mutex
	rt        node.Runtime
	remote    *iii.Client
	share     []string
	functions map[string]function // desired, by local function id
	refs      map[string]*iii.FunctionRef
}

func NewBridge(local caller, namespace string) *Bridge {
	return &Bridge{local: local, namespace: namespace, functions: map[string]function{}, refs: map[string]*iii.FunctionRef{}}
}

// SetRuntime must be a no-op for an unchanged runtime: the hub re-applies
// workers on every gateway auth, and a reconnect here would loop forever.
func (b *Bridge) SetRuntime(_ context.Context, rt node.Runtime) error {
	b.mu.Lock()
	defer b.mu.Unlock()
	if rt != b.rt {
		b.connect(rt)
	}
	return nil
}

// connect replaces the hub connection with one for rt. Callers hold b.mu.
func (b *Bridge) connect(rt node.Runtime) {
	b.closeRemote()
	b.rt = rt
	if rt.URL == "" {
		return
	}
	// An Authorization key with no value stops the SDK sending the local III_TOKEN to the hub.
	b.remote = iii.New(rt.URL, iii.WithOtel(iii.OtelConfig{Enabled: new(bool)}), iii.WithName("mesh-bridge"), iii.WithNamespace(rt.Namespace), iii.WithHeaders(http.Header{"Authorization": nil}))
	b.register()
	// A cancelled ctx starts the SDK's reconnect loop (and its iii ps marker)
	// before returning, so nothing of this client outlives Close.
	started, cancel := context.WithCancel(context.Background())
	cancel()
	_ = b.remote.Connect(started)
}

// Status reads the cached state only; the hub gives mesh-node::status 5 s.
func (b *Bridge) Status() []node.WorkerStatus {
	b.mu.Lock()
	defer b.mu.Unlock()
	out := []node.WorkerStatus{}
	for _, s := range b.shared("") {
		out = append(out, node.WorkerStatus{Name: s.Worker, State: s.State})
	}
	return out
}

// Shared lists the share entries with their hub function ids (<node>::<fn>).
func (b *Bridge) Shared(nodeName string) []SharedStatus {
	prefix := ""
	if nodeName != "" {
		prefix = nodeName + "::"
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.shared(prefix)
}

// Namespace is the hub namespace the bridge registers in, once the hub sent it.
func (b *Bridge) Namespace() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.rt.Namespace
}

func (b *Bridge) Close() {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.closeRemote()
	b.rt = node.Runtime{}
}

// Sync re-lists the shared workers' functions and updates the hub registrations.
// shortcut: 5s poll; observe engine::on-functions-changed if lag matters
func (b *Bridge) Sync(ctx context.Context, share []string) error {
	found := map[string]function{}
	var err error
	if len(share) > 0 { // an empty workers filter means "no filter" to the engine
		found, err = b.discover(ctx, share)
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	b.share = slices.Clone(share)
	if err != nil {
		// Keep what is known, but an unshared worker leaves the hub at once.
		maps.DeleteFunc(b.functions, func(_ string, f function) bool { return !slices.Contains(share, f.worker) })
	} else {
		b.functions = found
	}
	// A registration rejection (a stale twin still holding the worker name) stops
	// the SDK for good; retry on this tick, never on the hub's repeated applies.
	if b.remote != nil && b.remote.FatalError() != nil {
		b.connect(b.rt)
	}
	b.register()
	return err
}

type summary struct {
	FunctionID  string          `json:"function_id"`
	Namespace   string          `json:"namespace"`
	WorkerName  string          `json:"worker_name"`
	Description string          `json:"description"`
	Metadata    json.RawMessage `json:"metadata"`
}
type detail struct {
	FunctionID     string          `json:"function_id"`
	RequestSchema  json.RawMessage `json:"request_schema"`
	ResponseSchema json.RawMessage `json:"response_schema"`
	Error          string          `json:"error"`
}

func (b *Bridge) discover(ctx context.Context, share []string) (map[string]function, error) {
	var list struct {
		Functions []summary `json:"functions"`
	}
	if err := b.call(ctx, "engine::functions::list", map[string]any{"workers": share}, &list); err != nil {
		return nil, err
	}
	b.mu.Lock()
	known := maps.Clone(b.functions)
	b.mu.Unlock()
	found := map[string]function{}
	var fresh []string
	for _, row := range list.Functions {
		// The worker check also guards against an engine that ignores the workers filter.
		if row.Namespace != b.namespace || !slices.Contains(share, row.WorkerName) || !bridgeable(row.FunctionID) {
			continue
		}
		if f, ok := known[row.FunctionID]; ok && f.worker == row.WorkerName {
			found[row.FunctionID] = f
			continue
		}
		// Observable stays unset: false holds the registration until the hub
		// advertises observe.v2, which rbac-proxy before 1.1.0 strips.
		found[row.FunctionID] = function{worker: row.WorkerName, opts: iii.RegisterFunctionOptions{Description: row.Description, Metadata: row.Metadata}}
		fresh = append(fresh, row.FunctionID)
	}
	for chunk := range slices.Chunk(fresh, 32) { // engine cap per functions::info call
		var info struct {
			Functions []detail `json:"functions"`
		}
		if err := b.call(ctx, "engine::functions::info", map[string]any{"function_ids": chunk, "namespace": b.namespace}, &info); err != nil {
			return nil, err
		}
		for _, d := range info.Functions {
			if f, ok := found[d.FunctionID]; ok && d.Error == "" {
				f.opts.RequestFormat, f.opts.ResponseFormat = d.RequestSchema, d.ResponseSchema
				found[d.FunctionID] = f
			}
		}
	}
	return found, nil
}

func (b *Bridge) call(ctx context.Context, id string, payload, out any) error {
	body, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	raw, err := b.local.Call(ctx, iii.CallRequest{FunctionID: id, Payload: body, Timeout: 5 * time.Second})
	if err != nil {
		return fmt.Errorf("%s: %w", id, err)
	}
	return json.Unmarshal(raw, out)
}

var deniedPrefixes = []string{"engine::", "mesh::", "mesh-node::", "secrets::", "configuration::", "rbac-proxy::"}

func bridgeable(id string) bool {
	if !strings.Contains(id, "::") {
		return false
	}
	for _, p := range deniedPrefixes {
		if strings.HasPrefix(id, p) {
			return false
		}
	}
	return true
}

// register diffs the desired functions against the hub registrations. Changed
// schemas of an existing id are not re-registered. Callers hold b.mu.
func (b *Bridge) register() {
	if b.remote == nil {
		return
	}
	for id, ref := range b.refs {
		if _, ok := b.functions[id]; !ok {
			_ = ref.Unregister()
			delete(b.refs, id)
		}
	}
	for id, f := range b.functions {
		if b.refs[id] != nil {
			continue
		}
		ref, err := b.remote.RegisterFunction(id, b.forward(id), f.opts)
		if err != nil {
			log.Printf("mesh client: register %s: %v", id, err)
			continue
		}
		b.refs[id] = ref
	}
}

func (b *Bridge) closeRemote() {
	if b.remote != nil {
		b.remote.Close()
		b.remote = nil
	}
	b.refs = map[string]*iii.FunctionRef{}
}

func (b *Bridge) shared(prefix string) []SharedStatus {
	connected := b.remote != nil && b.remote.ConnectionState() == iii.StateConnected
	out := []SharedStatus{}
	for _, w := range b.share {
		s := SharedStatus{Worker: w, State: "not running", Functions: []string{}}
		registered := true
		for id, f := range b.functions {
			if f.worker == w {
				s.Functions = append(s.Functions, prefix+id)
				registered = registered && b.refs[id] != nil
			}
		}
		if len(s.Functions) > 0 {
			s.State = "pending"
			if connected && registered {
				s.State = "shared"
			}
		}
		slices.Sort(s.Functions)
		out = append(out, s)
	}
	return out
}

var errChannels = &iii.InvocationError{Code: "channels_unsupported", Message: "mesh client bridges request/response only"}

// forward runs a hub call of id on the local engine. Channel refs only work on
// the engine that created them, so they are refused both ways.
func (b *Bridge) forward(id string) iii.Handler {
	return func(ctx context.Context, data json.RawMessage) (any, error) {
		if hasChannelRefs(data) {
			return nil, errChannels
		}
		// shortcut: fixed 10m timeout and a hub-side cancel does not reach the local call; propagate both once the SDK carries cancellation
		out, err := b.local.Call(ctx, iii.CallRequest{FunctionID: id, Namespace: b.namespace, Payload: data, Timeout: 10 * time.Minute})
		if err != nil {
			return nil, err
		}
		if hasChannelRefs(out) {
			return nil, errChannels
		}
		return out, nil
	}
}

func hasChannelRefs(data json.RawMessage) bool {
	refs, err := iii.ExtractChannelRefs(data)
	return err == nil && len(refs) > 0
}

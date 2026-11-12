package client

import (
	"cmp"
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
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
)

type caller interface {
	Call(context.Context, iii.CallRequest) (json.RawMessage, error)
}

// localEngine is the local project's connection, which also serves the mirrors.
type localEngine interface {
	caller
	RegisterFunction(string, iii.Handler, ...iii.RegisterFunctionOptions) (*iii.FunctionRef, error)
}

// Function is a listed function with the options a copy of it registers with.
type Function struct {
	Worker string
	Opts   iii.RegisterFunctionOptions
}

// RemoteStatus lists what one owner shares with this machine, by local mirror id.
type RemoteStatus struct {
	Machine   string   `json:"machine"`
	Functions []string `json:"functions"`
}

// SharedStatus is one share entry in the local mesh::status.
type SharedStatus struct {
	Worker    string   `json:"worker"`
	State     string   `json:"state"`
	Functions []string `json:"functions"`
}

// Bridge is the client node's WorkerController: its workers are the shared
// local workers, whose functions it registers on one hub connection and
// forwards to the local engine. It also mirrors the mesh's catalog as local
// functions that call through the hub.
type Bridge struct {
	local     localEngine
	namespace string // local project namespace

	mu        sync.Mutex
	rt        node.Runtime
	remote    *iii.Client
	share     []string
	functions map[string]Function // desired, by local function id
	refs      map[string]*iii.FunctionRef
	retry     *time.Timer   // reconnects a connection the hub rejected
	backoff   time.Duration // wait before that reconnect
	catalog   node.Catalog
	mirrors   map[string]*iii.FunctionRef // local registrations, by mirror id
	online    context.Context             // cancelled when the hub connection leaves StateConnected
	offline   context.CancelFunc

	// Changed runs under the bridge's lock whenever Status or Shared may report something new; it must not block.
	Changed func()
}

// The first reconnect after the hub rejected the bridge; later ones wait up to a minute.
var bridgeRetry = 5 * time.Second

func (b *Bridge) changed() {
	if b.Changed != nil {
		b.Changed()
	}
}

func NewBridge(local localEngine, namespace string) *Bridge {
	return &Bridge{local: local, namespace: namespace, functions: map[string]Function{}, refs: map[string]*iii.FunctionRef{}, mirrors: map[string]*iii.FunctionRef{}}
}

// SetRuntime must be a no-op for an unchanged runtime: the hub re-applies
// workers on every gateway auth, and a reconnect here would loop forever.
func (b *Bridge) SetRuntime(_ context.Context, rt node.Runtime) error {
	b.mu.Lock()
	defer b.mu.Unlock()
	if rt != b.rt {
		// A leave, removal or rename: the node reports an empty revision and the hub sends the catalog again.
		b.setCatalog(node.Catalog{})
		b.connect(rt)
	}
	return nil
}

// connect replaces the hub connection with one for rt. Callers hold b.mu.
func (b *Bridge) connect(rt node.Runtime) {
	b.closeRemote()
	b.rt = rt
	defer b.changed()
	if rt.URL == "" {
		return
	}
	// An Authorization key with no value stops the SDK sending the local III_TOKEN to the hub.
	remote := iii.New(rt.URL, iii.WithOtel(iii.OtelConfig{Enabled: new(bool)}), iii.WithName("mesh-bridge"), iii.WithNamespace(rt.Namespace), iii.WithHeaders(http.Header{"Authorization": nil, meshnet.TokenHeader: {rt.Token}}))
	b.remote = remote
	b.register()
	// The SDK calls listeners on a goroutine of its own, never under Close, so this may take b.mu.
	remote.AddConnectionStateListener(func(state iii.ConnectionState) {
		b.mu.Lock()
		defer b.mu.Unlock()
		if b.remote != remote {
			return
		}
		if state != iii.StateConnected && b.offline != nil {
			b.offline()
			b.offline = nil
		} else if state == iii.StateConnected && b.offline == nil {
			b.online, b.offline = context.WithCancel(context.Background())
		}
		if state == iii.StateFailed {
			// A registration rejection (a stale twin still holding the worker name) stops the SDK for good.
			b.backoff = min(max(bridgeRetry, 2*b.backoff), time.Minute)
			b.retry = time.AfterFunc(b.backoff, func() {
				b.mu.Lock()
				defer b.mu.Unlock()
				if b.remote == remote {
					b.connect(b.rt)
				}
			})
		}
		b.changed()
	})
	// A cancelled ctx starts the SDK's reconnect loop (and its iii ps marker)
	// before returning, so nothing of this client outlives Close.
	started, cancel := context.WithCancel(context.Background())
	cancel()
	_ = b.remote.Connect(started)
	go func() {
		// Connected comes before the hub rejects the worker: only an accepted one ends the backoff.
		if remote.WaitUntilRegistered(context.Background()) == nil {
			b.mu.Lock()
			defer b.mu.Unlock()
			if b.remote == remote {
				b.backoff = 0
			}
		}
	}()
}

// Status reads the cached state only; the hub gives mesh-node::status 5 s.
func (b *Bridge) Status() []node.WorkerStatus {
	b.mu.Lock()
	defer b.mu.Unlock()
	out := []node.WorkerStatus{}
	for _, s := range b.shared("") {
		out = append(out, node.WorkerStatus{Name: s.Worker, State: s.State, Functions: s.Functions})
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
	b.setCatalog(node.Catalog{})
	b.rt = node.Runtime{}
}

// CatalogRev is the revision of the catalog last set.
func (b *Bridge) CatalogRev() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.catalog.Rev
}

// SetCatalog mirrors each entry another owner shares as a local function:
// hub::<id> for the hub's, peer::<machine>::<fn> for a machine's.
func (b *Bridge) SetCatalog(c node.Catalog) {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.setCatalog(c)
}

// setCatalog does not re-register a mirror whose schemas changed. Callers hold b.mu.
func (b *Bridge) setCatalog(c node.Catalog) {
	b.catalog = c
	want := map[string]node.CatalogEntry{}
	for _, e := range c.Functions {
		if e.Owner != b.rt.Node { // its own functions are local already
			want[mirrorID(e)] = e
		}
	}
	for id, ref := range b.mirrors {
		if _, ok := want[id]; !ok {
			_ = ref.Unregister()
			delete(b.mirrors, id)
		}
	}
	for id, e := range want {
		if b.mirrors[id] != nil {
			continue
		}
		// No metadata: what another machine sends must not mark a mirror internal or hidden.
		ref, err := b.local.RegisterFunction(id, b.mirror(e), iii.RegisterFunctionOptions{Description: e.Description, RequestFormat: e.RequestFormat, ResponseFormat: e.ResponseFormat})
		if err != nil {
			log.Printf("mesh client: mirror %q: %v", id, err)
			continue
		}
		b.mirrors[id] = ref
	}
	b.changed()
}

func mirrorID(e node.CatalogEntry) string {
	if e.Owner == "hub" {
		return "hub::" + e.ID
	}
	return "peer::" + e.ID // a machine's ids start with its name
}

// HubFirst orders owners: the hub, then machines by name.
func HubFirst(a, b string) int {
	rank := func(owner string) string {
		if owner == "hub" {
			return "" // machine names are never empty
		}
		return owner
	}
	return cmp.Compare(rank(a), rank(b))
}

// Remote lists the mirrors by owner, the hub first.
func (b *Bridge) Remote() []RemoteStatus {
	b.mu.Lock()
	defer b.mu.Unlock()
	byOwner := map[string][]string{}
	for _, e := range b.catalog.Functions {
		if e.Owner != b.rt.Node {
			byOwner[e.Owner] = append(byOwner[e.Owner], mirrorID(e))
		}
	}
	out := []RemoteStatus{}
	for _, owner := range slices.SortedFunc(maps.Keys(byOwner), HubFirst) {
		out = append(out, RemoteStatus{Machine: owner, Functions: slices.Sorted(slices.Values(byOwner[owner]))})
	}
	return out
}

var errOffline = &iii.InvocationError{Code: "mesh_offline", Message: "this machine is not connected to its hub"}

// mirror calls e through the hub's mesh::call. It ends when the hub connection
// drops: the SDK alone would leave the call waiting out its timeout.
func (b *Bridge) mirror(e node.CatalogEntry) iii.Handler {
	return func(ctx context.Context, data json.RawMessage) (any, error) {
		b.mu.Lock()
		remote, online, namespace := b.remote, b.online, b.rt.Namespace
		b.mu.Unlock()
		if remote == nil || online == nil || online.Err() != nil {
			return nil, errOffline
		}
		ctx, cancel := context.WithCancel(ctx)
		defer cancel()
		defer context.AfterFunc(online, cancel)()
		payload, err := json.Marshal(map[string]any{"owner": e.Owner, "function_id": e.ID, "payload": data})
		if err != nil {
			return nil, err
		}
		// 620 s outlasts the hub's 610 s and the owner's 600 s, so the nearest timeout reports.
		out, err := remote.Call(ctx, iii.CallRequest{FunctionID: "mesh::call", Namespace: namespace, Payload: payload, Timeout: 620 * time.Second})
		if err != nil && online.Err() != nil {
			return nil, errOffline
		}
		return out, err
	}
}

// Sync re-lists the shared workers' functions and updates the hub registrations.
func (b *Bridge) Sync(ctx context.Context, share []string) error {
	found := map[string]Function{}
	var err error
	if len(share) > 0 { // an empty workers filter means "no filter" to the engine
		b.mu.Lock()
		known, mirrors := maps.Clone(b.functions), maps.Clone(b.mirrors)
		b.mu.Unlock()
		found, err = Discover(ctx, b.local, b.namespace, share, func(r Row) bool {
			// The worker check also guards against an engine that ignores the workers filter.
			// A mirror is never published back, whatever the mesh worker is named.
			return r.Namespace == b.namespace && slices.Contains(share, r.WorkerName) && Bridgeable(r.FunctionID) && mirrors[r.FunctionID] == nil
		}, known)
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	b.share = slices.Clone(share)
	if err != nil {
		// Keep what is known, but an unshared worker leaves the hub at once.
		maps.DeleteFunc(b.functions, func(_ string, f Function) bool { return !slices.Contains(share, f.Worker) })
	} else {
		b.functions = found
	}
	// A rejected connection is replaced here too, never on the hub's repeated applies.
	if b.remote != nil && b.remote.FatalError() != nil {
		b.connect(b.rt)
	}
	b.register()
	b.changed()
	return err
}

// Row is one engine::functions::list entry.
type Row struct {
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

// Discover lists the functions of workers that keep accepts, with their schemas
// read in namespace. An id known under the same worker is reused without a read.
// workers must not be empty: the engine reads an empty filter as all workers.
func Discover(ctx context.Context, engine caller, namespace string, workers []string, keep func(Row) bool, known map[string]Function) (map[string]Function, error) {
	var list struct {
		Functions []Row `json:"functions"`
	}
	if err := call(ctx, engine, "engine::functions::list", map[string]any{"workers": workers}, &list); err != nil {
		return nil, err
	}
	found := map[string]Function{}
	var fresh []string
	for _, row := range list.Functions {
		if !keep(row) {
			continue
		}
		if f, ok := known[row.FunctionID]; ok && f.Worker == row.WorkerName {
			found[row.FunctionID] = f
			continue
		}
		// Observable stays unset: false holds the registration until the hub
		// advertises observe.v2, which rbac-proxy before 1.1.0 strips.
		found[row.FunctionID] = Function{Worker: row.WorkerName, Opts: iii.RegisterFunctionOptions{Description: row.Description, Metadata: row.Metadata}}
		fresh = append(fresh, row.FunctionID)
	}
	for chunk := range slices.Chunk(fresh, 32) { // engine cap per functions::info call
		var info struct {
			Functions []detail `json:"functions"`
		}
		if err := call(ctx, engine, "engine::functions::info", map[string]any{"function_ids": chunk, "namespace": namespace}, &info); err != nil {
			return nil, err
		}
		for _, d := range info.Functions {
			if f, ok := found[d.FunctionID]; ok && d.Error == "" {
				f.Opts.RequestFormat, f.Opts.ResponseFormat = d.RequestSchema, d.ResponseSchema
				found[d.FunctionID] = f
			}
		}
	}
	return found, nil
}

func call(ctx context.Context, engine caller, id string, payload, out any) error {
	body, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	raw, err := engine.Call(ctx, iii.CallRequest{FunctionID: id, Payload: body, Timeout: 5 * time.Second})
	if err != nil {
		return fmt.Errorf("%s: %w", id, err)
	}
	return json.Unmarshal(raw, out)
}

// approval:: is denied because the approval gate trusts any local approval:: caller:
// a copy of approval::resolve would let another machine approve this one's agent calls.
var deniedPrefixes = []string{"engine::", "mesh::", "mesh-node::", "secrets::", "configuration::", "rbac-proxy::", "approval::"}

// Bridgeable says whether id may be published to another project.
func Bridgeable(id string) bool {
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
		ref, err := b.remote.RegisterFunction(id, b.forward(id), f.Opts)
		if err != nil {
			log.Printf("mesh client: register %s: %v", id, err)
			continue
		}
		b.refs[id] = ref
	}
}

func (b *Bridge) closeRemote() {
	if b.offline != nil {
		b.offline()
		b.offline = nil
	}
	if b.retry != nil {
		b.retry.Stop()
		b.retry = nil
	}
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
			if f.Worker == w {
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
		if HasChannelRefs(data) {
			return nil, errChannels
		}
		// shortcut: fixed 10m timeout and a hub-side cancel does not reach the local call; propagate both once the SDK carries cancellation
		out, err := b.local.Call(ctx, iii.CallRequest{FunctionID: id, Namespace: b.namespace, Payload: data, Timeout: 10 * time.Minute})
		if err != nil {
			return nil, err
		}
		if HasChannelRefs(out) {
			return nil, errChannels
		}
		return out, nil
	}
}

// HasChannelRefs says whether data holds a channel ref, which works only on the engine that created it.
func HasChannelRefs(data json.RawMessage) bool {
	refs, err := iii.ExtractChannelRefs(data)
	return err == nil && len(refs) > 0
}

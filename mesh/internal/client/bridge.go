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

// localEngine is the local project's connection.
type localEngine interface {
	caller
}

// mirrorEngine is a local connection that serves one owner's mirrors in the namespace named after it.
type mirrorEngine interface {
	RegisterFunction(string, iii.Handler, ...iii.RegisterFunctionOptions) (*iii.FunctionRef, error)
	FatalError() error // the engine rejected the connection: another worker holds its name in that namespace
	Close() error
}

type mirrorSet struct {
	conn mirrorEngine
	refs map[string]*iii.FunctionRef // by function id
}

// Function is a listed function with the options a copy of it registers with.
type Function struct {
	Worker string
	Opts   iii.RegisterFunctionOptions
}

// RemoteStatus lists what one owner shares with this machine, by function id in the namespace named after the owner.
type RemoteStatus struct {
	Machine   string   `json:"machine"`
	Functions []string `json:"functions"`
	Path      string   `json:"path,omitempty"`    // how a machine's calls travel: hub, or peer to peer as direct, relay or idle
	Skipped   string   `json:"skipped,omitempty"` // why the owner is not mirrored here
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
// functions that call through the hub or peer to peer, each owner's on a local
// connection in a namespace named after the owner.
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
	dial      func(namespace string) mirrorEngine // opens a local connection; nil when the local engine has no address
	mirrors   map[string]*mirrorSet               // by owner
	skipped   map[string]string                   // owners of the catalog not mirrored, with the reason
	online    context.Context                     // cancelled when the hub connection leaves StateConnected
	offline   context.CancelFunc
	direct    *direct // the peer to peer path; nil while it is off

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
	b := &Bridge{local: local, namespace: namespace, functions: map[string]Function{}, refs: map[string]*iii.FunctionRef{}, mirrors: map[string]*mirrorSet{}, skipped: map[string]string{}}
	if a, ok := local.(interface{ Address() string }); ok {
		address := a.Address()
		// One worker name serves every owner: the engine's name lease is per namespace. III_TOKEN from the environment is right for the local engine.
		// shortcut: a mirror connection the engine rejects (another mesh client on this engine, or a twin of this worker, holds the name in that namespace) is dialed again only on the next catalog the hub sends. Give it the hub connection's retry when a local rejection is seen in practice.
		b.dial = func(namespace string) mirrorEngine {
			c := iii.New(address, iii.WithOtel(iii.OtelConfig{Enabled: new(bool)}), iii.WithName("mesh-mirror"), iii.WithNamespace(namespace))
			// As connect starts the hub connection: nothing of this client outlives Close.
			started, cancel := context.WithCancel(context.Background())
			cancel()
			_ = c.Connect(started)
			return c
		}
	}
	return b
}

// SetRuntime must be a no-op for an unchanged runtime: the hub re-applies
// workers on every gateway auth, and a reconnect here would loop forever.
func (b *Bridge) SetRuntime(_ context.Context, rt node.Runtime) error {
	b.mu.Lock()
	defer b.mu.Unlock()
	if rt != b.rt {
		// A leave, removal or rename: the node reports an empty revision and the hub sends the catalog again.
		b.setCatalog(node.Catalog{}, nil)
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
	// Its registrations live in the namespace named after this machine. Calls that name no
	// namespace would go there too, so mesh::call and mesh::node::changed name the hub's.
	remote := iii.New(rt.URL, iii.WithOtel(iii.OtelConfig{Enabled: new(bool)}), iii.WithName("mesh-bridge"), iii.WithNamespace(rt.Node), iii.WithHeaders(http.Header{"Authorization": nil, meshnet.TokenHeader: {rt.Token}}))
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
	for _, s := range b.shared() {
		out = append(out, node.WorkerStatus{Name: s.Worker, State: s.State, Functions: s.Functions})
	}
	return out
}

// Shared lists the share entries with their function ids, which the hub holds in the namespace named after this machine.
func (b *Bridge) Shared() []SharedStatus {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.shared()
}

// Namespace is the hub namespace the bridge calls mesh::call in, once the hub sent it.
func (b *Bridge) Namespace() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.rt.Namespace
}

// Announce tells the hub that what it shows of this machine changed, and reports whether it sent: not while the hub connection is down.
func (b *Bridge) Announce() bool {
	b.mu.Lock()
	remote, online, namespace := b.remote, b.online, b.rt.Namespace
	b.mu.Unlock()
	if remote == nil || online == nil || online.Err() != nil {
		return false
	}
	_, err := remote.Call(context.Background(), iii.CallRequest{FunctionID: "mesh::node::changed", Namespace: namespace, Action: iii.VoidAction()})
	return err == nil
}

func (b *Bridge) Close() {
	_ = b.SetDirect(nil, "")
	b.mu.Lock()
	defer b.mu.Unlock()
	b.closeRemote()
	b.setCatalog(node.Catalog{}, nil)
	b.rt = node.Runtime{}
}

// CatalogRev is the revision of the catalog last set.
func (b *Bridge) CatalogRev() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.catalog.Rev
}

// SetCatalog mirrors each entry another owner shares as a local function with
// its own id, in a namespace named after the owner. A failed read of the
// namespaces in use leaves the catalog as it was and returns the error, so the
// hub sends it again.
func (b *Bridge) SetCatalog(c node.Catalog) error {
	var used map[string]bool
	if len(c.Functions) > 0 {
		b.mu.Lock()
		ours := map[string]bool{}
		for owner, m := range b.mirrors {
			ours[owner] = m.conn.FatalError() == nil // a rejected connection holds nothing: the rows there are another worker's
		}
		b.mu.Unlock()
		var list struct {
			Functions []Row `json:"functions"`
		}
		if err := call(context.Background(), b.local, "engine::functions::list", map[string]any{"include_internal": true}, &list); err != nil {
			return fmt.Errorf("read the namespaces in use: %w", err)
		}
		used = map[string]bool{}
		for _, r := range list.Functions {
			// Another mesh client on this engine mirrors under the same worker name, and the engine leases that name per namespace.
			if r.WorkerName != "mesh-mirror" || !ours[r.Namespace] {
				used[r.Namespace] = true
			}
		}
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	b.setCatalog(c, used)
	return nil
}

// setCatalog does not re-register a mirror whose schemas or internal flag changed. Callers hold b.mu.
func (b *Bridge) setCatalog(c node.Catalog, used map[string]bool) {
	b.catalog = c
	byOwner := map[string]map[string]node.CatalogEntry{}
	for _, e := range c.Functions {
		if e.Owner != b.rt.Node { // its own functions are local already
			if byOwner[e.Owner] == nil {
				byOwner[e.Owner] = map[string]node.CatalogEntry{}
			}
			byOwner[e.Owner][e.ID] = e
		}
	}
	for owner, m := range b.mirrors {
		if byOwner[owner] == nil || m.conn.FatalError() != nil { // a rejected one is dialed again below while its namespace is free
			_ = m.conn.Close()
			delete(b.mirrors, owner)
		}
	}
	b.skipped = map[string]string{}
	for owner, want := range byOwner {
		m := b.mirrors[owner]
		// shortcut: checked on every catalog the hub sends, so a project that starts under a mirrored machine's name collides until the next catalog, and a namespace with no function yet is not seen. Read the listing again on engine::on-functions-changed when that happens in practice.
		if owner == b.namespace || used[owner] {
			if m != nil {
				_ = m.conn.Close()
				delete(b.mirrors, owner)
			}
			b.skipped[owner] = "namespace " + owner + " is already in use on this engine"
			continue
		}
		if b.dial == nil {
			b.skipped[owner] = "no local engine address"
			continue
		}
		if m == nil {
			m = &mirrorSet{conn: b.dial(owner), refs: map[string]*iii.FunctionRef{}}
			b.mirrors[owner] = m
		}
		for id, ref := range m.refs {
			if _, ok := want[id]; !ok {
				_ = ref.Unregister()
				delete(m.refs, id)
			}
		}
		for id, e := range want {
			if m.refs[id] != nil {
				continue
			}
			opts := iii.RegisterFunctionOptions{Description: e.Description, RequestFormat: e.RequestFormat, ResponseFormat: e.ResponseFormat}
			if e.Internal { // the only metadata another machine's entry carries: an internal function stays out of agent catalogs here too
				opts.Metadata = json.RawMessage(`{"internal":true}`)
			}
			ref, err := m.conn.RegisterFunction(id, b.mirror(e), opts)
			if err != nil {
				log.Printf("mesh client: mirror %q of %s: %v", id, owner, err)
				continue
			}
			m.refs[id] = ref
		}
	}
	b.changed()
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
	d := b.direct
	byOwner := map[string][]string{}
	for _, e := range b.catalog.Functions {
		if e.Owner != b.rt.Node {
			byOwner[e.Owner] = append(byOwner[e.Owner], e.ID)
		}
	}
	out := []RemoteStatus{}
	for _, owner := range slices.SortedFunc(maps.Keys(byOwner), HubFirst) {
		s := RemoteStatus{Machine: owner, Functions: slices.Sorted(slices.Values(byOwner[owner])), Skipped: b.skipped[owner]}
		if s.Skipped != "" {
			s.Functions = []string{}
		}
		out = append(out, s)
	}
	b.mu.Unlock()
	paths := map[string]string{}
	if d != nil {
		for _, p := range d.net.Peers() { // after the lock: this asks the backend
			paths[p.Name], _, _ = strings.Cut(p.Path, " ")
		}
	}
	for i := range out {
		if out[i].Machine != "hub" {
			out[i].Path = cmp.Or(paths[out[i].Machine], "hub") // a machine in the map is one that also turned peer to peer on
		}
	}
	return out
}

var errOffline = &iii.InvocationError{Code: "mesh_offline", Message: "this machine is not connected to its hub"}

// mirror calls e directly when its machine is in this machine's map, and with
// no fallback: a failed direct call may have run. Otherwise it calls through the
// hub's mesh::call, which ends when the hub connection drops: the SDK alone
// would leave the call waiting out its timeout.
func (b *Bridge) mirror(e node.CatalogEntry) iii.Handler {
	return func(ctx context.Context, data json.RawMessage) (any, error) {
		b.mu.Lock()
		remote, online, namespace, d := b.remote, b.online, b.rt.Namespace, b.direct
		b.mu.Unlock()
		if d != nil && e.Owner != "hub" {
			if addr, ok := d.net.Peer(e.Owner); ok { // in the map: both machines turned peer to peer on
				return d.call(ctx, e.Owner, addr, e.ID, data)
			}
		}
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

// Sync re-lists the shared functions and updates the hub registrations.
func (b *Bridge) Sync(ctx context.Context, share []string) error {
	found := map[string]Function{}
	var err error
	if len(share) > 0 { // an empty workers filter means "no filter" to the engine
		b.mu.Lock()
		known := maps.Clone(b.functions)
		b.mu.Unlock()
		ids, workers := SharesIDs(share), share
		if ids {
			workers = nil // an id names no worker: list them all
		}
		found, err = Discover(ctx, b.local, b.namespace, workers, ids, func(r Row) bool {
			// The entry check also guards against an engine that ignores the workers filter.
			// A mirror is never published back: it lives in its owner's namespace, never in this one.
			return r.Namespace == b.namespace && ShareEntry(share, r.WorkerName, r.FunctionID, Internal(r.Metadata)) != "" && Bridgeable(r.FunctionID)
		}, known)
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	b.share = slices.Clone(share)
	if err != nil {
		// Keep what is known, but an unshared worker or id leaves the hub at once.
		maps.DeleteFunc(b.functions, func(id string, f Function) bool {
			return ShareEntry(share, f.Worker, id, Internal(f.Opts.Metadata)) == ""
		})
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
// No workers lists every worker; internal adds the rows the engine hides by default.
func Discover(ctx context.Context, engine caller, namespace string, workers []string, internal bool, keep func(Row) bool, known map[string]Function) (map[string]Function, error) {
	var list struct {
		Functions []Row `json:"functions"`
	}
	req := map[string]any{}
	if len(workers) > 0 {
		req["workers"] = workers
	}
	if internal {
		req["include_internal"] = true
	}
	if err := call(ctx, engine, "engine::functions::list", req, &list); err != nil {
		return nil, err
	}
	found := map[string]Function{}
	var fresh []string
	for _, row := range list.Functions {
		if !keep(row) {
			continue
		}
		// A flipped internal flag reads a fresh copy: the keep rule and the status go by it.
		if f, ok := known[row.FunctionID]; ok && f.Worker == row.WorkerName && Internal(f.Opts.Metadata) == Internal(row.Metadata) {
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
// shortcut: nor a flipped internal flag (the hub row and mirrors keep the first until a restart); re-register on a flip if agent catalogs must follow it.
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

func (b *Bridge) shared() []SharedStatus {
	connected := b.remote != nil && b.remote.ConnectionState() == iii.StateConnected
	out := []SharedStatus{}
	for _, w := range b.share {
		s := SharedStatus{Worker: w, State: "not running", Functions: []string{}}
		registered := true
		for id, f := range b.functions {
			if ShareEntry(b.share, f.Worker, id, Internal(f.Opts.Metadata)) == w {
				s.Functions = append(s.Functions, id)
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
		data = DropMachineBound(id, data)
		// shortcut: fixed 10m timeout and a hub-side cancel does not reach the local call; propagate both once the SDK carries cancellation
		out, err := b.local.Call(ctx, iii.CallRequest{FunctionID: id, Namespace: b.namespace, Payload: data, Timeout: 10 * time.Minute})
		if err != nil {
			return nil, DropAccessRequest(err)
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

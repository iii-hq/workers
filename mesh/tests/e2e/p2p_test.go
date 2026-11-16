//go:build e2e

package e2e

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"hash/crc32"
	"net/http"
	"net/netip"
	"net/url"
	"os"
	"path/filepath"
	"runtime/pprof"
	"slices"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	meshclient "github.com/iii-hq/workers/mesh/internal/client"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"tailscale.com/feature/buildfeatures"
	"tailscale.com/tailcfg"
)

// machine is one client: its own stack, a real node and a real bridge whose local project is a namespace of the fixture's engine.
type machine struct {
	*testNode
	bridge *meshclient.Bridge
	local  string
}

func (f *fixture) machine(name string) *machine {
	t := f.t
	t.Helper()
	m := &machine{testNode: f.enrollMesh(name), local: name + "-local"}
	m.bridge = meshclient.NewBridge(connect(t, f.ctx, f.engineURL, "mesh", m.local), m.local)
	t.Cleanup(m.bridge.Close)
	m.mesh.OnChange(m.bridge.MapChanged) // the hook the client's runner sets
	f.startNode(m.testNode, m.bridge)
	return m
}

// path is how the machine's status says it reaches another one.
func (m *machine) path(to string) string {
	for _, r := range m.bridge.Remote() {
		if r.Machine == to {
			return r.Path
		}
	}
	return ""
}

// invoked counts the calls of id that the machine's hub connections carried in direction; with no id, every call but the hub's own node management.
func (n *testNode) invoked(direction, id string) (count int) {
	for _, frame := range n.snapshot() {
		got, _ := frame.Data["function_id"].(string)
		if frame.Direction == direction && frame.Data["type"] == "invokefunction" && (got == id || id == "" && !strings.HasPrefix(got, "mesh-node::")) {
			count++
		}
	}
	return count
}

// raw sends one direct call from the machine's stack, as a machine that skips its own bridge would, and returns the status and the refusal's code.
func (m *machine) raw(t *testing.T, to netip.Addr, function, version string, payload any) string {
	t.Helper()
	body, _ := json.Marshal(payload)
	req, err := http.NewRequestWithContext(t.Context(), http.MethodPost, "http://"+netip.AddrPortFrom(to, meshnet.PeerPort).String()+"/call?function="+url.QueryEscape(function), bytes.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("X-Mesh-Version", version)
	req.Header.Set("Expect", "100-continue") // a refusal that crosses the payload is lost with the connection
	client := &http.Client{Timeout: 20 * time.Second, Transport: &http.Transport{DialContext: m.mesh.Dial, DisableKeepAlives: true, ExpectContinueTimeout: 10 * time.Second}}
	res, err := client.Do(req)
	if err != nil {
		t.Fatalf("direct call of %s: %v", function, err)
	}
	defer res.Body.Close()
	var refusal struct{ Code string }
	_ = json.NewDecoder(res.Body).Decode(&refusal)
	return strings.TrimSpace(fmt.Sprint(res.StatusCode, " ", refusal.Code))
}

// dial gives up after 3 s: a flow the packet filter or WireGuard drops gets no answer at all.
func dial(m *meshnet.Mesh, to netip.Addr, port uint16) error {
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	c, err := m.Dial(ctx, "tcp", netip.AddrPortFrom(to, port).String())
	if err == nil {
		c.Close()
	}
	return err
}

// peerAPIPort is the port of a machine's backend peer API: the one the hub stored, which its map gives the machine's peers, when this build serves it, else the network library's first choice for addr.
func (f *fixture) peerAPIPort(t *testing.T, name string, addr netip.Addr) uint16 {
	a := addr.As16()
	api := 32768 | uint16(crc32.ChecksumIEEE(a[13:]))
	if !buildfeatures.HasPeerAPIServer {
		return api
	}
	eventually(t, 10*time.Second, name+"'s peer API port at the hub", func() error {
		var state struct {
			Nodes []struct {
				Name     string
				Hostinfo *tailcfg.Hostinfo
			}
		}
		b, _ := os.ReadFile(filepath.Join(f.dir, "hub", "state.json"))
		_ = json.Unmarshal(b, &state)
		for _, n := range state.Nodes {
			if n.Name == name && n.Hostinfo != nil {
				for _, s := range n.Hostinfo.Services {
					if s.Proto == tailcfg.PeerAPI6 {
						api = s.Port
						return nil
					}
				}
			}
		}
		return errors.New("not stored")
	})
	return api
}

// serving is whether a machine of this process is inside a direct call's handler, whose return gives the caller's slot back.
// shortcut: reads the goroutine dump because the bridge does not export its calls in flight, read that count once it does
func serving() bool {
	var b bytes.Buffer
	_ = pprof.Lookup("goroutine").WriteTo(&b, 1)
	return strings.Contains(b.String(), "client.(*direct).serveCall")
}

// Two machines that both turn peer to peer on call each other with no hub engine function in between, and each still refuses what it does not share.
func TestE2EPeerToPeer(t *testing.T) {
	const version = "0.1.0" // the fixture's hub and nodes run it
	f := newFixture(t, "project")
	var echoes, greetings, denied atomic.Int32
	waiting, release := make(chan struct{}, 1), make(chan struct{})
	defer close(release)
	answer := func(c *iii.Client, id string, run func(context.Context) string) {
		t.Helper()
		if _, err := iii.RegisterFunctionTyped(c, id, func(ctx context.Context, _ json.RawMessage) (map[string]string, error) {
			return map[string]string{"node": run(ctx)}, nil
		}); err != nil {
			t.Fatal(err)
		}
	}
	never := func(context.Context) string { denied.Add(1); return "beta" }
	answer(connect(t, f.ctx, f.engineURL, "greeter", f.ns), "greeter::hello", func(context.Context) string { greetings.Add(1); return "hub" })
	cfg := f.config
	cfg.Share = []string{"greeter"}
	mustCall(t, f.admin, "default", "configuration::set", map[string]any{"id": "mesh", "value": cfg})

	// Beta's project: a shared worker with a fast function, one that waits and two ids no machine may share, and a worker it does not share.
	probe := connect(t, f.ctx, f.engineURL, "probe", "beta-local")
	answer(probe, "probe::echo", func(context.Context) string { echoes.Add(1); return "beta" })
	answer(probe, "probe::wait", func(ctx context.Context) string {
		waiting <- struct{}{}
		select {
		case <-release:
		case <-ctx.Done():
		}
		return "beta"
	})
	answer(probe, "mesh::status", never)
	answer(probe, "approval::resolve", never)
	answer(connect(t, f.ctx, f.engineURL, "hidden", "beta-local"), "hidden::peek", never)

	beta, alpha := f.machine("beta"), f.machine("alpha")
	// share sets beta's share list and waits until it holds exactly the ids in want.
	share := func(workers []string, want ...string) {
		t.Helper()
		eventually(t, 10*time.Second, "beta's share list", func() error {
			if err := beta.bridge.Sync(f.ctx, workers); err != nil {
				return err
			}
			got := []string{}
			for _, w := range beta.bridge.Status() {
				got = append(got, w.Functions...)
			}
			if !slices.Equal(got, want) {
				return fmt.Errorf("beta shares %v", got)
			}
			return nil
		})
	}
	// mirrored is whether the engine lists every id in a machine's project.
	mirrored := func(m *machine, ids ...string) error {
		out, err := call(f.ctx, f.admin, "default", "engine::functions::list", map[string]any{})
		var list struct{ Functions []meshclient.Row }
		if err == nil {
			err = json.Unmarshal(out, &list)
		}
		for _, id := range ids {
			if err == nil && !slices.ContainsFunc(list.Functions, func(r meshclient.Row) bool { return r.Namespace == m.local && r.FunctionID == id }) {
				err = fmt.Errorf("%s has no %s", m.local, id)
			}
		}
		return err
	}
	ask := func(id string) (string, error) {
		// 20 s: the first packets between two machines can wait 5 s for a WireGuard retry.
		out, err := f.admin.Call(f.ctx, iii.CallRequest{FunctionID: id, Namespace: alpha.local, Payload: json.RawMessage(`{}`), Timeout: 20 * time.Second})
		var reply struct{ Node string }
		if err == nil {
			err = json.Unmarshal(out, &reply)
		}
		return reply.Node, err
	}
	// echo calls beta's echo through alpha's mirror and returns what it added to the hub connections: alpha's mesh::call frames and the calls beta took.
	echo := func() (sent, taken int) {
		t.Helper()
		sent, taken = alpha.invoked("node-to-hub", "mesh::call"), beta.invoked("hub-to-node", "")
		ran := echoes.Load()
		if node, err := ask("peer::beta::probe::echo"); err != nil || node != "beta" || echoes.Load() != ran+1 {
			t.Fatalf("peer::beta::probe::echo answered %q after %d runs: %v", node, echoes.Load()-ran, err)
		}
		return alpha.invoked("node-to-hub", "mesh::call") - sent, beta.invoked("hub-to-node", "") - taken
	}
	// on opens a machine's direct path, as its runner does for client.peer_to_peer.
	on := func(m *machine) func() error {
		return func() error { return m.bridge.SetDirect(m.mesh, version) }
	}
	seen := func(want bool) func() error {
		return func() error {
			_, a := alpha.mesh.Peer("beta")
			_, b := beta.mesh.Peer("alpha")
			if a != want || b != want {
				return fmt.Errorf("alpha has beta in its map: %v, beta has alpha: %v", a, b)
			}
			return nil
		}
	}

	share([]string{"probe"}, "probe::echo", "probe::wait")
	eventually(t, 30*time.Second, "the mirrors of what beta and the hub share", func() error {
		return errors.Join(mirrored(alpha, "peer::beta::probe::echo", "peer::beta::probe::wait", "hub::greeter::hello"), mirrored(beta, "hub::greeter::hello"))
	})

	// Off at both: the hub carries the call.
	if sent, taken := echo(); sent != 1 || taken != 1 || alpha.path("beta") != "hub" {
		t.Fatalf("with peer to peer off: %d calls sent to the hub, %d taken from it, path %q", sent, taken, alpha.path("beta"))
	}

	// On at both: each enters the other's map.
	for _, m := range []*machine{beta, alpha} {
		if err := on(m)(); err != nil {
			t.Fatal(err)
		}
	}
	eventually(t, 20*time.Second, "the two machines to see each other", seen(true))
	to := beta.mesh.Self()
	if p := alpha.path("beta"); !slices.Contains([]string{"direct", "relay", "idle"}, p) {
		t.Fatalf("alpha reaches beta by %q", p)
	}
	// The proof: the same call answers and neither hub connection carried it.
	if sent, taken := echo(); sent != 0 || taken != 0 {
		t.Fatalf("with peer to peer on: %d calls sent to the hub, %d taken from it", sent, taken)
	}
	t.Logf("alpha reaches beta by %q", alpha.path("beta"))

	// What a machine that skips its own bridge gets from beta's port.
	ran := echoes.Load()
	refs := map[string]any{"reader": map[string]any{"channel_id": "c", "access_key": "k", "direction": "read"}}
	for _, c := range []struct {
		function, version string
		payload           any
		want              string
	}{
		{"probe::echo", version, map[string]any{}, "200"},
		{"hidden::peek", version, map[string]any{}, "403 FORBIDDEN"},
		{"mesh::status", version, map[string]any{}, "403 FORBIDDEN"},
		{"approval::resolve", version, map[string]any{}, "403 FORBIDDEN"},
		{"hub::greeter::hello", version, map[string]any{}, "403 FORBIDDEN"}, // a mirror: beta is no way to the hub
		{"probe::echo", "0.0.9", map[string]any{}, "403 FORBIDDEN"},
		{"probe::echo", version, refs, "400 channels_unsupported"},
	} {
		if got := alpha.raw(t, to, c.function, c.version, c.payload); got != c.want {
			t.Fatalf("direct call of %s as %s with %v: %s, want %s", c.function, c.version, c.payload, got, c.want)
		}
	}
	if echoes.Load() != ran+1 || denied.Load() != 0 || greetings.Load() != 0 {
		t.Fatalf("refused calls ran: echo %d times, unshared functions %d, the hub's %d", echoes.Load()-ran, denied.Load(), greetings.Load())
	}
	// A machine with the setting off reaches no other machine, and the port is all a peer reaches.
	gamma := f.enrollMesh("gamma")
	if err := dial(gamma.mesh, to, meshnet.PeerPort); err == nil {
		t.Fatal("gamma, with peer to peer off, opened beta's port")
	}
	_, known := beta.mesh.Peer("gamma")
	if _, knows := gamma.mesh.Peer("beta"); known || knows { // read after the dial, so a map that showed them had time to arrive
		t.Fatalf("beta has gamma in its map: %v, gamma has beta: %v", known, knows)
	}
	api := f.peerAPIPort(t, "beta", to)
	if err := dial(alpha.mesh, to, api); err == nil {
		t.Fatalf("alpha opened beta's peer API port %d", api)
	}

	// Beta's own list decides, with no hub round trip: an unshare is refused at the next call.
	share(nil)
	if got := alpha.raw(t, to, "probe::echo", version, map[string]any{}); got != "403 FORBIDDEN" || echoes.Load() != ran+1 {
		t.Fatalf("direct call of an unshared probe::echo: %s, %d runs", got, echoes.Load()-ran-1)
	}
	eventually(t, 30*time.Second, "alpha's mirror of the unshared echo to go", func() error {
		if mirrored(alpha, "peer::beta::probe::echo") == nil {
			return errors.New("still mirrored")
		}
		return nil
	})
	share([]string{"probe"}, "probe::echo", "probe::wait")
	eventually(t, 30*time.Second, "alpha's mirrors of beta's probe again", func() error {
		return mirrored(alpha, "peer::beta::probe::echo", "peer::beta::probe::wait")
	})
	if sent, taken := echo(); sent != 0 || taken != 0 {
		t.Fatalf("shared again: %d calls sent to the hub, %d taken from it", sent, taken)
	}

	// Beta turns it off: alpha's map drops beta and the same mirror goes through the hub again.
	if err := beta.bridge.SetDirect(nil, ""); err != nil {
		t.Fatal(err)
	}
	eventually(t, 20*time.Second, "the two machines to lose each other", seen(false))
	if sent, taken := echo(); sent != 1 || taken != 1 || alpha.path("beta") != "hub" {
		t.Fatalf("with peer to peer off at beta: %d calls sent to the hub, %d taken from it, path %q", sent, taken, alpha.path("beta"))
	}
	// Back on. Beta's stack holds the port for a minute after the last connection it took.
	began := time.Now()
	eventually(t, 3*time.Minute, "beta's port to open again", on(beta))
	t.Logf("beta's port opened again after %s", time.Since(began).Round(time.Second))
	eventually(t, 20*time.Second, "the two machines to see each other again", seen(true))
	if sent, taken := echo(); sent != 0 || taken != 0 {
		t.Fatalf("with peer to peer on again: %d calls sent to the hub, %d taken from it", sent, taken)
	}

	// The hub path dies: the proxy is gone, the coordinator and the relay run in this process and stay.
	if node, err := ask("hub::greeter::hello"); err != nil || node != "hub" {
		t.Fatalf("hub::greeter::hello answered %q: %v", node, err)
	}
	if err := syscall.Kill(-f.proxy.Process.Pid, syscall.SIGKILL); err != nil {
		t.Fatal(err)
	}
	eventually(t, 30*time.Second, "alpha's hub mirror to go offline", func() error {
		if node, err := ask("hub::greeter::hello"); err == nil || !strings.Contains(err.Error(), "mesh_offline") {
			return fmt.Errorf("hub::greeter::hello answered %q: %v", node, err)
		}
		return nil
	})
	if sent, taken := echo(); sent != 0 || taken != 0 {
		t.Fatalf("with the hub path dead: %d calls sent to the hub, %d taken from it", sent, taken)
	}

	// The hub removes alpha during a call: both ends drop it at once, and beta has no keepalive to tell it otherwise.
	from, done := alpha.mesh.Self(), make(chan error, 1)
	go func() { _, err := ask("peer::beta::probe::wait"); done <- err }()
	select {
	case <-waiting:
	case err := <-done:
		t.Fatalf("peer::beta::probe::wait returned: %v", err)
	}
	if !serving() {
		t.Fatal("beta runs a direct call and no handler is seen")
	}
	mustCall(t, f.admin, f.ns, "mesh::nodes::remove", map[string]any{"name": "alpha"})
	select {
	case err := <-done:
		if err == nil || !strings.Contains(err.Error(), "mesh_offline") {
			t.Fatalf("the call of a removed machine ended with: %v", err)
		}
	case <-time.After(15 * time.Second):
		t.Fatal("the call of a removed machine did not end")
	}
	eventually(t, 15*time.Second, "beta to drop alpha", func() error {
		_, listed := beta.mesh.Peer("alpha")
		if _, named := beta.mesh.WhoIs(netip.AddrPortFrom(from, 0)); listed || named {
			return fmt.Errorf("alpha in beta's map: %v, its address named: %v", listed, named)
		}
		return nil
	})
	// The engine runs the function to its end, so beta's handler, and the slot, wait for it.
	if !serving() {
		t.Fatal("beta's handler returned while its function still runs")
	}
	select {
	case release <- struct{}{}:
	case <-time.After(15 * time.Second):
		t.Fatal("probe::wait is no longer waiting")
	}
	eventually(t, 15*time.Second, "beta's handler to return", func() error {
		if serving() {
			return errors.New("a handler still running")
		}
		return nil
	})
	if node, err := ask("peer::beta::probe::echo"); err == nil {
		t.Fatalf("a removed machine's call answered %q", node)
	}
}

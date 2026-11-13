package coordinator

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/json"
	"net/http/httptest"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/join"
	"tailscale.com/tailcfg"
	"tailscale.com/tstest"
	"tailscale.com/tstime"
	"tailscale.com/types/key"
)

func issueToken(t *testing.T, c *Coordinator, opts join.TokenOptions) (string, string, string) {
	t.Helper()
	id, raw, err := c.IssueToken("https://hub.example", opts)
	if err != nil {
		t.Fatal(err)
	}
	envelope, err := join.DecodeToken(raw)
	if err != nil {
		t.Fatal(err)
	}
	return id, raw, envelope.Secret
}
func tokenRequest(secret, name string) tailcfg.RegisterRequest {
	return tailcfg.RegisterRequest{NodeKey: key.NewNode().Public(), Auth: &tailcfg.RegisterResponseAuth{AuthKey: secret}, Hostinfo: &tailcfg.Hostinfo{Hostname: name}}
}
func TestSingleUseTokenConsumed(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	_, raw, secret := issueToken(t, c, join.TokenOptions{})
	for range 2 {
		if _, err := c.RedeemToken(secret); err != nil {
			t.Fatal("preflight consumed token", err)
		}
	}
	var accepted atomic.Int32
	var wg sync.WaitGroup
	for range 12 {
		wg.Go(func() {
			if c.register(key.NewMachine().Public(), "127.0.0.1", tokenRequest(secret, "runner")).MachineAuthorized {
				accepted.Add(1)
			}
		})
	}
	wg.Wait()
	if accepted.Load() != 1 || len(c.Nodes()) != 1 {
		t.Fatalf("accepted %d nodes %d", accepted.Load(), len(c.Nodes()))
	}
	if _, err := c.RedeemToken(secret); err == nil {
		t.Fatal("consumed token accepted")
	}
	b, _ := os.ReadFile(c.file)
	if bytes.Contains(b, []byte(secret)) || bytes.Contains(b, []byte(raw)) {
		t.Fatal("credential leaked into coordinator state")
	}
	n := c.st.Nodes[0]
	if !c.register(n.Machine, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: n.Key}).MachineAuthorized {
		t.Fatal("consumed token interrupted accepted node")
	}
}
func TestExpiredAndRevokedRefused(t *testing.T) {
	clock := asyncClock{tstest.NewClock(tstest.ClockOpts{})}
	c, err := newWithClock(t.TempDir(), func(string, string) (string, error) { return "", nil }, clock)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	id, _, secret := issueToken(t, c, join.TokenOptions{ExpiresInSecs: 60, Ephemeral: true, Reusable: true})
	accepted := tokenRequest(secret, "accepted")
	machine := key.NewMachine().Public()
	if !c.register(machine, "a", accepted).MachineAuthorized {
		t.Fatal("initial registration failed")
	}
	code := c.Code()
	clock.Advance(time.Minute)
	if !c.register(machine, "a", tailcfg.RegisterRequest{NodeKey: accepted.NodeKey}).MachineAuthorized || c.Code() != code {
		t.Fatal("token expiry affected accepted node or device code")
	}
	if c.register(key.NewMachine().Public(), "a", tokenRequest(secret, "expired")).MachineAuthorized {
		t.Fatal("expired token accepted")
	}
	id, _, secret = issueToken(t, c, join.TokenOptions{Ephemeral: true, Reusable: true})
	if err := c.RevokeToken(id); err != nil {
		t.Fatal(err)
	}
	if c.register(key.NewMachine().Public(), "b", tokenRequest(secret, "revoked")).MachineAuthorized {
		t.Fatal("revoked token accepted")
	}
}
func TestRemoveRevokesCreatingToken(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	_, _, secret := issueToken(t, c, join.TokenOptions{Ephemeral: true, Reusable: true})
	req := tokenRequest(secret, "runner")
	if !c.register(key.NewMachine().Public(), "a", req).MachineAuthorized {
		t.Fatal("join failed")
	}
	if !c.Nodes()[0].Ephemeral {
		t.Fatal("token ephemeral policy ignored")
	}
	if err := c.Remove("runner"); err != nil {
		t.Fatal(err)
	}
	c.Close()
	c = newCoordinator(t, dir)
	if c.register(key.NewMachine().Public(), "b", tokenRequest(secret, "fresh")).MachineAuthorized {
		t.Fatal("removed token accepted fresh keys after restart")
	}
}
func TestTokenTransactionRollback(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	id, _, secret := issueToken(t, c, join.TokenOptions{})
	file := c.file
	c.file = filepath.Join(file, "impossible")
	before := len(c.st.Tokens)
	if id, raw, err := c.IssueToken("https://hub.example", join.TokenOptions{}); err == nil || id != "" || raw != "" || len(c.st.Tokens) != before {
		t.Fatal("failed issuance returned a credential or changed token authority")
	}
	if c.register(key.NewMachine().Public(), "a", tokenRequest(secret, "runner")).MachineAuthorized {
		t.Fatal("failed save authorized node")
	}
	if _, err := c.RedeemToken(secret); err != nil {
		t.Fatal("failed save consumed token", err)
	}
	if err := c.RevokeToken(id); err == nil {
		t.Fatal("failed revoke succeeded")
	}
	if _, err := c.RedeemToken(secret); err != nil {
		t.Fatal("failed save revoked token", err)
	}
	c.file = file
	req := tokenRequest(secret, "runner")
	machine := key.NewMachine().Public()
	if !c.register(machine, "a", req).MachineAuthorized {
		t.Fatal("retry failed")
	}
	c.file = filepath.Join(file, "impossible")
	code := c.Code()
	if err := c.Remove("runner"); err == nil {
		t.Fatal("failed removal succeeded")
	}
	if !c.Known(req.NodeKey) || c.Code() != code {
		t.Fatal("failed removal changed authority")
	}
	c.file = file
	c.Close()
	c = newCoordinator(t, filepath.Dir(file))
	if !c.Known(req.NodeKey) || c.register(key.NewMachine().Public(), "b", tokenRequest(secret, "again")).MachineAuthorized {
		t.Fatal("restart lost node or token consumption")
	}
}
func TestDeviceCodeUnaffectedByRevoke(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	code := c.Code()
	id, _, _ := issueToken(t, c, join.TokenOptions{})
	if err := c.RevokeToken(id); err != nil {
		t.Fatal(err)
	}
	if c.Code() != code {
		t.Fatal("token revoke cycled device code")
	}
}
func TestEphemeralExpiresAfter3Min(t *testing.T) {
	clock := asyncClock{tstest.NewClock(tstest.ClockOpts{})}
	dir := t.TempDir()
	c, err := newWithClock(dir, func(string, string) (string, error) { return "", nil }, clock)
	if err != nil {
		t.Fatal(err)
	}
	_, _, secret := issueToken(t, c, join.TokenOptions{Ephemeral: true, Reusable: true})
	req := tokenRequest(secret, "runner")
	machine := key.NewMachine().Public()
	if !c.register(machine, "a", req).MachineAuthorized {
		t.Fatal("join failed")
	}
	clock.Advance(2 * time.Minute)
	b, _ := json.Marshal(tailcfg.MapRequest{NodeKey: req.NodeKey})
	c.serveMap(httptest.NewRecorder(), httptest.NewRequest("POST", "/machine/map", bytes.NewReader(b)), machine)
	clock.Advance(2 * time.Minute)
	if !c.Known(req.NodeKey) {
		t.Fatal("expired before last map deadline")
	}
	c.Close()
	c, err = newWithClock(dir, func(string, string) (string, error) { return "", nil }, clock)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	gone := make(chan string, 1)
	c.SetGone(func(name string) { gone <- name })
	clock.Advance(time.Minute)
	select {
	case name := <-gone:
		if name != "runner" {
			t.Fatal(name)
		}
	case <-time.After(time.Second):
		t.Fatal("restart extended expiry past last map deadline")
	}
	if c.Known(req.NodeKey) || len(c.Nodes()) != 0 {
		t.Fatal("expired node remains authorized")
	}
	if _, err := c.RedeemToken(secret); err != nil {
		t.Fatal("expiry revoked reusable token", err)
	}
}

// tstest runs callbacks inline under its clock lock; time.AfterFunc does not.
type asyncClock struct{ *tstest.Clock }

func (c asyncClock) AfterFunc(d time.Duration, f func()) tstime.TimerController {
	return c.Clock.AfterFunc(d, func() { go f() })
}

func TestEphemeralActivePollSurvivesRestart(t *testing.T) {
	clock := asyncClock{tstest.NewClock(tstest.ClockOpts{})}
	dir := t.TempDir()
	c, err := newWithClock(dir, func(string, string) (string, error) { return "", nil }, clock)
	if err != nil {
		t.Fatal(err)
	}
	_, _, secret := issueToken(t, c, join.TokenOptions{Ephemeral: true})
	req := tokenRequest(secret, "runner")
	machine := key.NewMachine().Public()
	if !c.register(machine, "a", req).MachineAuthorized {
		t.Fatal("join failed")
	}
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	b, _ := json.Marshal(tailcfg.MapRequest{NodeKey: req.NodeKey, Stream: true})
	recorder := &pollRecorder{ResponseRecorder: httptest.NewRecorder(), flushed: make(chan struct{}, 10)}
	done := make(chan struct{})
	go func() {
		defer close(done)
		c.serveMap(recorder, httptest.NewRequest("POST", "/machine/map", bytes.NewReader(b)).WithContext(ctx), machine)
	}()
	waitFlush := func() {
		t.Helper()
		select {
		case <-recorder.flushed:
		case <-time.After(time.Second):
			t.Fatal("map did not progress with fake clock")
		}
	}
	waitFlush()
	for range 5 {
		clock.Advance(time.Minute)
		waitFlush()
		deadline := time.Now().Add(time.Second)
		for {
			c.mu.Lock()
			last := c.byKey(req.NodeKey).LastMap
			c.mu.Unlock()
			if last.Equal(clock.Now()) {
				break
			}
			if time.Now().After(deadline) {
				t.Fatal("map progress not persisted")
			}
			time.Sleep(time.Millisecond)
		}
	}
	cancel()
	<-done
	c.Close()
	c, err = newWithClock(dir, func(string, string) (string, error) { return "", nil }, clock)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	clock.Advance(time.Minute)
	if !c.Known(req.NodeKey) {
		t.Fatal("healthy long-running map was purged on restart")
	}
	if !c.register(machine, "a", tailcfg.RegisterRequest{NodeKey: req.NodeKey}).MachineAuthorized {
		t.Fatal("restart required consumed token")
	}
	gone := make(chan string, 1)
	c.SetGone(func(name string) { gone <- name })
	clock.Advance(2 * time.Minute)
	select {
	case <-gone:
	case <-time.After(time.Second):
		t.Fatal("registration reset last-map expiry")
	}
}

type pollRecorder struct {
	*httptest.ResponseRecorder
	flushed chan struct{}
}

func (w *pollRecorder) Flush() { w.ResponseRecorder.Flush(); w.flushed <- struct{}{} }

// meshnet's 10-minute rule counts from the last response a machine received, so each one says when the hub wrote it.
func TestEveryMapResponseCarriesTheTime(t *testing.T) {
	clock := asyncClock{tstest.NewClock(tstest.ClockOpts{})}
	c, err := newWithClock(t.TempDir(), func(string, string) (string, error) { return "", nil }, clock)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	_, _, secret := issueToken(t, c, join.TokenOptions{})
	req := tokenRequest(secret, "laptop")
	machine := key.NewMachine().Public()
	if !c.register(machine, "a", req).MachineAuthorized {
		t.Fatal("join failed")
	}
	ctx, cancel := context.WithCancel(t.Context())
	b, _ := json.Marshal(tailcfg.MapRequest{NodeKey: req.NodeKey, Stream: true})
	recorder := &pollRecorder{ResponseRecorder: httptest.NewRecorder(), flushed: make(chan struct{}, 10)}
	done := make(chan struct{})
	go func() {
		defer close(done)
		c.serveMap(recorder, httptest.NewRequest("POST", "/machine/map", bytes.NewReader(b)).WithContext(ctx), machine)
	}()
	defer func() { cancel(); <-done }()
	check := func(what string, keepAlive bool) {
		t.Helper()
		select {
		case <-recorder.flushed:
		case <-time.After(time.Second):
			t.Fatalf("%s was not written", what)
		}
		var resp tailcfg.MapResponse
		size := binary.LittleEndian.Uint32(recorder.Body.Next(4))
		if err := json.Unmarshal(recorder.Body.Next(int(size)), &resp); err != nil {
			t.Fatalf("%s: %v", what, err)
		}
		if resp.KeepAlive != keepAlive || resp.ControlTime == nil || !resp.ControlTime.Equal(clock.Now()) {
			t.Fatalf("%s: keepalive %t with time %v, want %t with %s", what, resp.KeepAlive, resp.ControlTime, keepAlive, clock.Now())
		}
	}
	check("the full map", false)
	clock.Advance(time.Second) // short of the keepalive, so the next response is the delta
	c.mu.Lock()
	c.changed(c.byKey(req.NodeKey))
	c.mu.Unlock()
	check("a delta", false)
	clock.Advance(time.Minute)
	check("a keepalive", true)
}

func TestEphemeralNameReuseRevokesOnlyCurrentToken(t *testing.T) {
	clock := asyncClock{tstest.NewClock(tstest.ClockOpts{})}
	c, err := newWithClock(t.TempDir(), func(string, string) (string, error) { return "", nil }, clock)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	_, _, old := issueToken(t, c, join.TokenOptions{Ephemeral: true, Reusable: true})
	if !c.register(key.NewMachine().Public(), "a", tokenRequest(old, "runner")).MachineAuthorized {
		t.Fatal("first join failed")
	}
	gone := make(chan string, 2)
	c.SetGone(func(name string) { gone <- name })
	clock.Advance(3 * time.Minute)
	select {
	case <-gone:
	case <-time.After(time.Second):
		t.Fatal("node did not expire")
	}
	_, _, current := issueToken(t, c, join.TokenOptions{Ephemeral: true, Reusable: true})
	if !c.register(key.NewMachine().Public(), "b", tokenRequest(current, "runner")).MachineAuthorized {
		t.Fatal("second join failed")
	}
	if err := c.Remove("runner"); err != nil {
		t.Fatal(err)
	}
	if _, err := c.RedeemToken(old); err != nil {
		t.Fatal("removal revoked historic token of same name", err)
	}
	if _, err := c.RedeemToken(current); err == nil {
		t.Fatal("removal left current token valid")
	}
}

func TestTokenPreflightCannotRaceRevokeOrCodeCycle(t *testing.T) {
	for _, operation := range []string{"revoke", "cycle"} {
		t.Run(operation, func(t *testing.T) {
			c := newCoordinator(t, t.TempDir())
			id, _, secret := issueToken(t, c, join.TokenOptions{})
			ready, resume := make(chan struct{}), make(chan struct{})
			c.join = func(secret, addr string) (string, error) {
				_, err := c.RedeemToken(secret)
				close(ready)
				<-resume
				return "", err
			}
			done := make(chan tailcfg.RegisterResponse, 1)
			go func() { done <- c.register(key.NewMachine().Public(), "a", tokenRequest(secret, "runner")) }()
			<-ready
			if operation == "revoke" {
				if err := c.RevokeToken(id); err != nil {
					t.Fatal(err)
				}
			} else {
				if err := c.CycleCode(); err != nil {
					t.Fatal(err)
				}
			}
			close(resume)
			got := <-done
			if got.MachineAuthorized != (operation == "cycle") {
				t.Fatalf("%s registration authorized=%t error=%s", operation, got.MachineAuthorized, got.Error)
			}
		})
	}
}
func TestTokenRemovalRollbackKeepsReusableCredential(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	_, _, secret := issueToken(t, c, join.TokenOptions{Ephemeral: true, Reusable: true})
	req := tokenRequest(secret, "runner")
	if !c.register(key.NewMachine().Public(), "a", req).MachineAuthorized {
		t.Fatal("join failed")
	}
	file := c.file
	c.file = filepath.Join(file, "impossible")
	if err := c.Remove("runner"); err == nil {
		t.Fatal("removal should fail")
	}
	if _, err := c.RedeemToken(secret); err != nil {
		t.Fatal("failed removal revoked reusable token", err)
	}
	c.file = file
	if !c.register(key.NewMachine().Public(), "b", tokenRequest(secret, "runner-two")).MachineAuthorized {
		t.Fatal("reusable token unusable after rollback")
	}
}
func TestEphemeralExpiryRemovesPeerFromHubMap(t *testing.T) {
	clock := asyncClock{tstest.NewClock(tstest.ClockOpts{})}
	c, err := newWithClock(t.TempDir(), func(string, string) (string, error) { return "", nil }, clock)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	registerNode(t, c, "hub")
	_, _, secret := issueToken(t, c, join.TokenOptions{Ephemeral: true, Reusable: true})
	req := tokenRequest(secret, "runner")
	if !c.register(key.NewMachine().Public(), "a", req).MachineAuthorized {
		t.Fatal("join failed")
	}
	sent, host := map[tailcfg.NodeID]uint64{}, ""
	c.delta(c.byName("hub"), sent, &host, true, nil)
	id := c.byName("runner").ID
	gone := make(chan string, 1)
	c.SetGone(func(name string) { gone <- name })
	clock.Advance(3 * time.Minute)
	select {
	case <-gone:
	case <-time.After(time.Second):
		t.Fatal("expiry hook missing")
	}
	delta, _ := c.delta(c.byName("hub"), sent, &host, false, nil)
	if delta == nil || !slices.Contains(delta.PeersRemoved, id) {
		t.Fatal("expired peer not removed from map")
	}
}

func TestTokenRefusalsShareDeviceCodeRateLimit(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	limit := join.NewLimit(10, 10*time.Minute, time.Now)
	c.SetJoinLimit(limit)
	id, _, secret := issueToken(t, c, join.TokenOptions{})
	if err := c.RevokeToken(id); err != nil {
		t.Fatal(err)
	}
	for range 10 {
		if c.register(key.NewMachine().Public(), "192.0.2.1", tokenRequest(secret, "refused")).MachineAuthorized {
			t.Fatal("revoked token accepted")
		}
	}
	req := httptest.NewRequest("POST", "/join", strings.NewReader("{}"))
	req.RemoteAddr = "192.0.2.1:1234"
	w := httptest.NewRecorder()
	join.Handler(c.Identity(), c.Code, limit, nil).ServeHTTP(w, req)
	if w.Code != 429 {
		t.Fatalf("token refusals did not throttle code exchange: %d", w.Code)
	}
	_, _, valid := issueToken(t, c, join.TokenOptions{})
	if c.register(key.NewMachine().Public(), "192.0.2.1", tokenRequest(valid, "throttled")).MachineAuthorized {
		t.Fatal("token bypassed shared limit")
	}
	if !c.register(key.NewMachine().Public(), "192.0.2.2", tokenRequest(valid, "allowed")).MachineAuthorized {
		t.Fatal("limit affected other client")
	}
}

func TestInvalidTokenStateRejected(t *testing.T) {
	for _, damage := range []string{"hash", "duplicate", "policy", "node-owner"} {
		t.Run(damage, func(t *testing.T) {
			dir := t.TempDir()
			c := newCoordinator(t, dir)
			_, _, secret := issueToken(t, c, join.TokenOptions{})
			if !c.register(key.NewMachine().Public(), "a", tokenRequest(secret, "runner")).MachineAuthorized {
				t.Fatal("join failed")
			}
			c.Close()
			switch damage {
			case "hash":
				c.st.Tokens[0].Hash = "invalid"
			case "duplicate":
				c.st.Tokens = append(c.st.Tokens, c.st.Tokens[0])
			case "policy":
				c.st.Tokens[0].Reusable = true
			case "node-owner":
				c.st.Nodes[0].TokenID = "missing"
			}
			if err := c.save(); err != nil {
				t.Fatal(err)
			}
			reopened, err := New(dir, nil)
			if err == nil {
				reopened.Close()
				t.Fatal("accepted corrupt token authority")
			}
		})
	}
}
func TestRevokeForNodePersistsWithoutRemovingNode(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	_, _, secret := issueToken(t, c, join.TokenOptions{Ephemeral: true, Reusable: true})
	req := tokenRequest(secret, "runner")
	if !c.register(key.NewMachine().Public(), "a", req).MachineAuthorized {
		t.Fatal("join failed")
	}
	code := c.Code()
	if err := c.RevokeForNode("runner"); err != nil {
		t.Fatal(err)
	}
	if !c.Known(req.NodeKey) || c.Code() != code {
		t.Fatal("revoke removed accepted node or rotated code")
	}
	c.Close()
	c = newCoordinator(t, dir)
	if _, err := c.RedeemToken(secret); err == nil {
		t.Fatal("node credential revocation lost on restart")
	}
}

func TestExpiredRestartInstallsHookConcurrently(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	_, _, secret := issueToken(t, c, join.TokenOptions{Ephemeral: true})
	req := tokenRequest(secret, "runner")
	if !c.register(key.NewMachine().Public(), "a", req).MachineAuthorized {
		t.Fatal("join failed")
	}
	c.Close()
	c.st.Nodes[0].JoinedAt = time.Now().Add(-4 * time.Minute)
	if err := c.save(); err != nil {
		t.Fatal(err)
	}
	c = newCoordinator(t, dir)
	for range 100 {
		c.SetGone(func(string) {})
	}
	deadline := time.After(time.Second)
	for c.Known(req.NodeKey) {
		select {
		case <-deadline:
			t.Fatal("expired node revived on restart")
		case <-time.After(time.Millisecond):
		}
	}
}

func TestTokenAcceptedMachineRotatesKeys(t *testing.T) {
	for _, credential := range []string{"consumed", "revoked", "expired"} {
		t.Run(credential, func(t *testing.T) {
			clock := asyncClock{tstest.NewClock(tstest.ClockOpts{})}
			c, err := newWithClock(t.TempDir(), nil, clock)
			if err != nil {
				t.Fatal(err)
			}
			defer c.Close()
			c.join = func(secret, addr string) (string, error) { _, err := c.RedeemToken(secret); return "", err }
			opts := join.TokenOptions{}
			if credential != "consumed" {
				opts = join.TokenOptions{Ephemeral: true, Reusable: true, ExpiresInSecs: 60}
			}
			id, _, secret := issueToken(t, c, opts)
			machine := key.NewMachine().Public()
			first := tokenRequest(secret, "runner")
			if r := c.register(machine, "a", first); !r.MachineAuthorized {
				t.Fatal(r.Error)
			}
			if err := c.SetRuntimeMetadata("runner", RuntimeMetadata{OS: "linux", Arch: "amd64", Version: "0.1.0"}); err != nil {
				t.Fatal(err)
			}
			if credential == "revoked" {
				if err := c.RevokeToken(id); err != nil {
					t.Fatal(err)
				}
			}
			if credential == "expired" {
				clock.Advance(time.Minute)
			}
			if _, err := c.RedeemToken(secret); err == nil {
				t.Fatal("credential still permits enrollment")
			}
			before := *c.byMachine(machine)
			tokens, _ := json.Marshal(c.st.Tokens)
			if r := c.register(machine, "a", tailcfg.RegisterRequest{NodeKey: first.NodeKey}); !r.MachineAuthorized {
				t.Fatal("same-key reconnect failed", r.Error)
			}
			if r := c.register(key.NewMachine().Public(), "a", tokenRequest(secret, "runner")); r.MachineAuthorized {
				t.Fatal("fresh machine reused enrollment credential")
			}
			previous := first.NodeKey
			for _, withCredential := range []bool{true, false} {
				req := tailcfg.RegisterRequest{NodeKey: key.NewNode().Public(), OldNodeKey: previous, Hostinfo: &tailcfg.Hostinfo{Hostname: "changed-hostname"}, Ephemeral: !opts.Ephemeral}
				if withCredential {
					req.Auth = &tailcfg.RegisterResponseAuth{AuthKey: secret}
				}
				if r := c.register(machine, "a", req); !r.MachineAuthorized {
					t.Fatalf("accepted machine cannot rotate with credential=%t: %s", withCredential, r.Error)
				}
				current := c.byMachine(machine)
				if current.ID != before.ID || current.Name != before.Name || current.TokenID != id || current.Ephemeral != before.Ephemeral || current.RuntimeMetadata != before.RuntimeMetadata || current.JoinedAt != before.JoinedAt || !slices.Contains(current.Keys, previous) || current.Key != req.NodeKey || len(c.Nodes()) != 1 {
					t.Fatal("rotation replaced accepted-node identity, history or provenance")
				}
				if c.Nodes()[0].Addr != c.address(before.ID) {
					t.Fatal("rotation changed address")
				}
				after, _ := json.Marshal(c.st.Tokens)
				if !bytes.Equal(tokens, after) {
					t.Fatal("rotation changed enrollment token authority")
				}
				previous = req.NodeKey
			}
			_, _, other := issueToken(t, c, join.TokenOptions{Ephemeral: true, Reusable: true})
			attacker := key.NewMachine().Public()
			foreign := tokenRequest(other, "other")
			if r := c.register(attacker, "b", foreign); !r.MachineAuthorized {
				t.Fatal(r.Error)
			}
			stolen := tailcfg.RegisterRequest{NodeKey: first.NodeKey, OldNodeKey: foreign.NodeKey}
			if r := c.register(attacker, "b", stolen); r.MachineAuthorized || r.Error != "node key registered to another machine" {
				t.Fatal("accepted machine stole foreign historical key", r.Error)
			}
			if err := c.Remove("runner"); err != nil {
				t.Fatal(err)
			}
			for _, req := range []tailcfg.RegisterRequest{tokenRequest(other, "runner"), {NodeKey: previous, Auth: &tailcfg.RegisterResponseAuth{AuthKey: other}}} {
				if r := c.register(machine, "a", req); r.MachineAuthorized || r.Error != "node keys refused" {
					t.Fatal("removed machine reauthorized", r.Error)
				}
			}
			if r := c.register(key.NewMachine().Public(), "b", tailcfg.RegisterRequest{NodeKey: first.NodeKey, Auth: &tailcfg.RegisterResponseAuth{AuthKey: other}}); r.MachineAuthorized || r.Error != "node keys refused" {
				t.Fatal("removed historical key reauthorized", r.Error)
			}
		})
	}
}

func TestTokenRotationSaveFailureKeepsAcceptedKey(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	c.join = func(secret, addr string) (string, error) { _, err := c.RedeemToken(secret); return "", err }
	_, _, secret := issueToken(t, c, join.TokenOptions{})
	machine := key.NewMachine().Public()
	first := tokenRequest(secret, "runner")
	if r := c.register(machine, "a", first); !r.MachineAuthorized {
		t.Fatal(r.Error)
	}
	next := tailcfg.RegisterRequest{NodeKey: key.NewNode().Public(), OldNodeKey: first.NodeKey}
	file := c.file
	c.file = filepath.Join(file, "impossible")
	if r := c.register(machine, "a", next); r.Error == "" || r.MachineAuthorized {
		t.Fatal("failed save accepted rotation")
	}
	if !c.Known(first.NodeKey) || c.Known(next.NodeKey) || slices.Contains(c.byMachine(machine).Keys, next.NodeKey) {
		t.Fatal("failed rotation changed accepted key history")
	}
	c.file = file
	if r := c.register(machine, "a", next); !r.MachineAuthorized {
		t.Fatal("accepted machine could not retry rotation", r.Error)
	}
	c.Close()
	c = newCoordinator(t, dir)
	if !c.Known(next.NodeKey) || c.Known(first.NodeKey) || len(c.Nodes()) != 1 {
		t.Fatal("rotation authority lost on restart")
	}
}

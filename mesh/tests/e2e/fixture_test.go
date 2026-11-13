//go:build e2e

package e2e

import (
	"context"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/hub"
	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"tailscale.com/tsnet"
)

type fixture struct {
	t                                                  *testing.T
	ctx                                                context.Context
	dir, ns, engineURL, proxyURL, frontURL, hubAddress string
	admin                                              *iii.Client
	proxy                                              *exec.Cmd // kept so a test can kill it
	config                                             hub.Config
	identity                                           ed25519.PrivateKey
}

func freePort(t *testing.T) int {
	t.Helper()
	l, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer l.Close()
	return l.Addr().(*net.TCPAddr).Port
}

func binary(t *testing.T, env string) string {
	t.Helper()
	p := os.Getenv(env)
	if p == "" {
		t.Fatalf("%s is required; run ./run-tests.sh", env)
	}
	p, err := filepath.Abs(p)
	if err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(p)
	if err != nil || info.Mode()&0111 == 0 {
		t.Fatalf("%s=%s is not executable: %v", env, p, err)
	}
	return p
}

func startProcess(t *testing.T, dir, name, bin string, env []string, args ...string) *exec.Cmd {
	t.Helper()
	logPath := filepath.Join(dir, name+".log")
	log, err := os.OpenFile(logPath, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0600)
	if err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(bin, args...)
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	cmd.Dir = dir
	cmd.Env = append(append(os.Environ(), "III_COMPOSE_DIR="+dir), env...)
	cmd.Stdout = log
	cmd.Stderr = log
	if err := cmd.Start(); err != nil {
		log.Close()
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
		_ = cmd.Wait()
		_ = log.Close()
		b, _ := os.ReadFile(logPath)
		if reports := os.Getenv("MESH_E2E_REPORT_DIR"); reports != "" {
			_ = os.WriteFile(filepath.Join(reports, strings.ReplaceAll(t.Name(), "/", "-")+"-"+name+".log"), b, 0600)
		}
		if t.Failed() {
			if len(b) > 16000 {
				b = b[len(b)-16000:]
			}
			t.Logf("%s (tail):\n%s", name, b)
		}
	})
	return cmd
}

func writeJSON(t *testing.T, path string, value any) {
	t.Helper()
	b, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(path, b, 0600); err != nil {
		t.Fatal(err)
	}
}
func connect(t *testing.T, ctx context.Context, url, name, ns string) *iii.Client {
	t.Helper()
	c := iii.New(url, iii.WithOtel(iii.OtelConfig{Enabled: new(bool)}), iii.WithName(name), iii.WithNamespace(ns))
	t.Cleanup(func() { c.Close() })
	wait, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	if err := c.Connect(wait); err != nil {
		t.Fatal(err)
	}
	return c
}
func call(ctx context.Context, c *iii.Client, ns, id string, p any) (json.RawMessage, error) {
	b, err := json.Marshal(p)
	if err != nil {
		return nil, err
	}
	return c.Call(ctx, iii.CallRequest{FunctionID: id, Namespace: ns, Payload: b, Timeout: time.Second})
}
func mustCall(t *testing.T, c *iii.Client, ns, id string, p any) json.RawMessage {
	t.Helper()
	out, err := call(t.Context(), c, ns, id, p)
	if err != nil {
		t.Fatalf("%s [%s]: %v", id, ns, err)
	}
	return out
}
func eventually(t *testing.T, duration time.Duration, what string, check func() error) {
	t.Helper()
	deadline := time.Now().Add(duration)
	var err error
	for time.Now().Before(deadline) {
		if err = check(); err == nil {
			return
		}
		time.Sleep(50 * time.Millisecond)
	}
	t.Fatalf("%s: %v", what, err)
}
func newFixture(t *testing.T, ns string) *fixture {
	t.Helper()
	t.Setenv("TS_NO_LOGS_NO_SUPPORT", "true")
	t.Setenv("III_NAMESPACE", ns)
	ctx, cancel := context.WithCancel(t.Context())
	t.Cleanup(cancel)
	f := &fixture{t: t, ctx: ctx, ns: ns, dir: t.TempDir()}
	enginePort, proxyPort, frontPort := freePort(t), freePort(t), freePort(t)
	f.engineURL = fmt.Sprintf("ws://127.0.0.1:%d", enginePort)
	f.proxyURL = fmt.Sprintf("ws://127.0.0.1:%d", proxyPort)
	f.frontURL = fmt.Sprintf("http://127.0.0.1:%d", frontPort)
	configPath := filepath.Join(f.dir, "engine.json")
	writeJSON(t, configPath, map[string]any{"workers": []any{map[string]any{"name": "iii-worker-manager", "config": map[string]any{"host": "127.0.0.1", "port": enginePort}}}})
	startProcess(t, f.dir, "engine", binary(t, "III_BIN"), []string{"III_NAMESPACE=default"}, "--no-update-check", "--config", configPath)
	f.admin = connect(t, ctx, f.engineURL, "e2e-admin", ns)
	eventually(t, 20*time.Second, "configuration ready", func() error {
		_, err := call(ctx, f.admin, "default", "configuration::ensure", map[string]any{"id": "e2e", "name": "e2e", "description": "e2e", "schema": map[string]any{}, "initial_value": map[string]any{}})
		return err
	})
	f.config = hub.DefaultConfig()
	f.config.Listen = fmt.Sprintf("127.0.0.1:%d", frontPort)
	f.config.PublicURL = f.frontURL
	f.config.ProxyURL = f.proxyURL
	mustCall(t, f.admin, "default", "configuration::ensure", map[string]any{"id": "mesh", "name": "Mesh", "description": "mesh", "schema": map[string]any{}, "initial_value": f.config})
	seed := filepath.Join(f.dir, "proxy.json")
	writeJSON(t, seed, map[string]any{"host": "127.0.0.1", "port": proxyPort, "engine_url": f.engineURL})
	f.proxy = startProcess(t, f.dir, "proxy", binary(t, "MESH_TEST_PROXY_BIN"), nil, "--url", f.engineURL, "--config", seed)
	eventually(t, 20*time.Second, "proxy ready", func() error { _, err := call(ctx, f.admin, ns, "rbac-proxy::status", map[string]any{}); return err })
	hubDir := filepath.Join(f.dir, "hub")
	done := make(chan error, 1)
	go func() {
		err := hub.Run(ctx, hub.Options{URL: f.engineURL, Version: "0.1.0", DataDir: hubDir})
		if err != nil {
			t.Logf("hub Run returned: %v", err)
		}
		done <- err
	}()
	t.Cleanup(func() {
		cancel()
		select {
		case err := <-done:
			if err != nil {
				t.Errorf("hub: %v", err)
			}
		case <-time.After(10 * time.Second):
			t.Error("hub did not stop")
		}
	})
	eventually(t, 30*time.Second, "hub gateway ready", func() error {
		select {
		case err := <-done:
			done <- err
			t.Fatalf("hub exited: %v", err)
		default:
		}
		out, err := call(ctx, f.admin, ns, "mesh::status", map[string]any{})
		if err != nil {
			return err
		}
		var status hub.StatusResult
		if err = json.Unmarshal(out, &status); err != nil {
			return err
		}
		if status.Proxy != "ok" || status.Gateway != "ok" {
			return fmt.Errorf("status %+v", status)
		}
		f.hubAddress = net.JoinHostPort(status.HubAddress, "49135")
		return nil
	})
	b, err := os.ReadFile(filepath.Join(hubDir, "state.json"))
	if err != nil {
		t.Fatal(err)
	}
	var state struct {
		Identity ed25519.PrivateKey `json:"identity"`
	}
	if err = json.Unmarshal(b, &state); err != nil {
		t.Fatal(err)
	}
	f.identity = state.Identity
	return f
}

type observedFrame struct {
	Connection int
	Direction  string
	Data       map[string]any
}
type testNode struct {
	server      *tsnet.Server
	mesh        *meshnet.Mesh // the client's own stack, in place of server (enrollMesh)
	name, url   string
	mu          sync.Mutex
	frames      []observedFrame
	connections int
}

func (n *testNode) snapshot() []observedFrame {
	n.mu.Lock()
	defer n.mu.Unlock()
	return append([]observedFrame(nil), n.frames...)
}

// redeem spends a fresh join code for the secret and the hub's verified identity.
func (f *fixture) redeem() (string, join.Identity) {
	t := f.t
	t.Helper()
	out := mustCall(t, f.admin, f.ns, "mesh::join::code", map[string]any{})
	var code hub.JoinResult
	if err := json.Unmarshal(out, &code); err != nil {
		t.Fatal(err)
	}
	secret, identity, err := join.Redeem(f.ctx, f.frontURL, code.Code)
	if err != nil {
		t.Fatal(err)
	}
	doc, err := join.FetchIdentity(f.ctx, f.frontURL)
	if err != nil {
		t.Fatal(err)
	}
	if err = doc.Verify(identity); err != nil {
		t.Fatal(err)
	}
	return secret, doc
}
func (f *fixture) enroll(name string) *testNode {
	t := f.t
	t.Helper()
	secret, doc := f.redeem()
	shim, err := meshnet.NewShim(f.frontURL, doc.Control)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { shim.Close() })
	n := &testNode{name: name, server: &tsnet.Server{Dir: t.TempDir(), Hostname: name, ControlURL: shim.URL(), AuthKey: secret, Logf: func(string, ...any) {}, UserLogf: func(string, ...any) {}}}
	t.Cleanup(func() { n.server.Close() })
	ctx, cancel := context.WithTimeout(f.ctx, 20*time.Second)
	defer cancel()
	if _, err = n.server.Up(ctx); err != nil {
		t.Fatal(err)
	}
	f.forward(n, n.server.Dial)
	return n
}

// enrollMesh joins name with the client's own stack, which two machines need to call each other.
func (f *fixture) enrollMesh(name string) *testNode {
	t := f.t
	t.Helper()
	secret, doc := f.redeem()
	m, err := meshnet.UpUserspace(t.TempDir(), f.frontURL, secret, name, doc.Control)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { m.Close() })
	eventually(t, 20*time.Second, name+" joined", func() error {
		if got, _ := m.SelfName(); got != name {
			return fmt.Errorf("named %q", got)
		}
		if _, ok := m.Peer("hub"); !ok {
			return errors.New("the hub is not in its map")
		}
		return nil
	})
	n := &testNode{name: name, mesh: m}
	f.forward(n, m.Dial)
	return n
}

// forward gives n the loopback URL that reaches the hub gateway through dial and records every frame.
func (f *fixture) forward(n *testNode, dial func(context.Context, string, string) (net.Conn, error)) {
	t := f.t
	// Child processes cannot dial an in-process userspace stack. Each forwarding
	// connection dials through this node, so the real gateway still owns identity.
	transport := &http.Transport{DialContext: dial}
	t.Cleanup(transport.CloseIdleConnections)
	forwarding := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		upstream, _, err := websocket.Dial(r.Context(), "ws://"+f.hubAddress, &websocket.DialOptions{HTTPClient: &http.Client{Transport: transport}})
		if err != nil {
			http.Error(w, err.Error(), 502)
			return
		}
		defer upstream.CloseNow()
		downstream, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer downstream.CloseNow()
		upstream.SetReadLimit(16 << 20)
		downstream.SetReadLimit(16 << 20)
		n.mu.Lock()
		n.connections++
		connection := n.connections
		n.mu.Unlock()
		ctx, cancel := context.WithCancel(r.Context())
		defer cancel()
		relay := func(dst, src *websocket.Conn, direction string) {
			defer cancel()
			for {
				kind, data, err := src.Read(ctx)
				if err != nil {
					return
				}
				var frame map[string]any
				if json.Unmarshal(data, &frame) == nil {
					n.mu.Lock()
					n.frames = append(n.frames, observedFrame{Connection: connection, Direction: direction, Data: frame})
					n.mu.Unlock()
				}
				if dst.Write(ctx, kind, data) != nil {
					return
				}
			}
		}
		go relay(upstream, downstream, "node-to-hub")
		relay(downstream, upstream, "hub-to-node")
	}))
	t.Cleanup(forwarding.Close)
	n.url = "ws" + strings.TrimPrefix(forwarding.URL, "http")
}

func mustJSON(v any) string { b, _ := json.Marshal(v); return string(b) }

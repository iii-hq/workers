//go:build e2e

package e2e

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/netip"
	"os"
	"os/user"
	"path/filepath"
	"strings"
	"time"

	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
	"github.com/iii-hq/workers/mesh/internal/supervisor"
)

type roundTrip func(*http.Request) (*http.Response, error)

func (r roundTrip) RoundTrip(req *http.Request) (*http.Response, error) { return r(req) }

type nodeNetwork struct{ n *testNode }

func (n nodeNetwork) Self() netip.Addr {
	ip, _ := n.n.server.TailscaleIPs()
	if ip.IsValid() {
		return ip
	}
	_, ip = n.n.server.TailscaleIPs()
	return ip
}
func (n nodeNetwork) SelfName() (string, bool)        { return n.n.name, true }
func (n nodeNetwork) Peers() []meshnet.PeerStatus     { return nil }
func (n nodeNetwork) GatewayPort() (uint16, bool)     { return 49135, true }
func (n nodeNetwork) GatewayChanges() <-chan struct{} { return nil }
func (n nodeNetwork) Removed() <-chan struct{}        { return nil }

type forwardedController struct {
	node.WorkerController
	url string
}

func (c forwardedController) SetRuntime(ctx context.Context, rt node.Runtime) error {
	rt.URL = c.url
	return c.WorkerController.SetRuntime(ctx, rt)
}

func (f *fixture) startNode(n *testNode, withIDE bool) *supervisor.Supervisor {
	if withIDE {
		return f.startWorker(n, "ide", "0.12.36", "MESH_TEST_IDE_BIN")
	}
	return f.startWorker(n, "", "", "")
}

func (f *fixture) startWorker(n *testNode, worker, version, binaryEnv string) *supervisor.Supervisor {
	t := f.t
	t.Helper()
	u, err := user.Current()
	if err != nil {
		t.Fatal(err)
	}
	operator, err := supervisor.LookupOperator(u.Username)
	if err != nil {
		t.Fatal(err)
	}
	cache := filepath.Join(f.dir, n.name+"-cache")
	if err = os.MkdirAll(cache, 0755); err != nil {
		t.Fatal(err)
	}
	if worker != "" {
		b, err := os.ReadFile(binary(t, binaryEnv))
		if err != nil {
			t.Fatal(err)
		}
		digest := fmt.Sprintf("%x", sha256.Sum256(b))
		if err = os.WriteFile(filepath.Join(cache, digest), b, 0555); err != nil {
			t.Fatal(err)
		}
		original := http.DefaultClient
		transport := original.Transport
		if transport == nil {
			transport = http.DefaultTransport
		}
		http.DefaultClient = &http.Client{Transport: roundTrip(func(req *http.Request) (*http.Response, error) {
			if req.URL.Host != "api.workers.iii.dev" {
				return transport.RoundTrip(req)
			}
			if req.Method != "POST" || req.URL.Path != "/resolve" {
				return nil, fmt.Errorf("unexpected registry route: %s %s", req.Method, req.URL.Path)
			}
			var request struct{ Worker, Version, Target string }
			if err := json.NewDecoder(req.Body).Decode(&request); err != nil {
				return nil, err
			}
			if request.Worker != worker || request.Version != version {
				return nil, fmt.Errorf("unexpected registry request: %+v", request)
			}
			body := mustJSON(map[string]any{"graph": []any{map[string]any{"name": worker, "version": version, "type": "binary", "binaries": map[string]any{request.Target: map[string]any{"url": "https://artifacts.example.invalid/" + worker, "sha256": digest}}}}})
			return &http.Response{StatusCode: 200, Body: io.NopCloser(strings.NewReader(body)), Header: make(http.Header)}, nil
		})}
		f.restoreHTTP = func() { http.DefaultClient = original }
		cfg := f.config.Nodes[n.name]
		cfg.Workers = []string{worker + "@" + version}
		if cfg.Allow == nil {
			cfg.Allow = []string{}
		}
		f.config.Nodes[n.name] = cfg
		f.reload()
	}
	s := supervisor.New(cache, filepath.Join(f.dir, n.name+"-logs"), operator, n.name, "")
	t.Cleanup(func() {
		s.Close()
		if reports := os.Getenv("MESH_E2E_REPORT_DIR"); reports != "" {
			b, _ := os.ReadFile(filepath.Join(f.dir, n.name+"-logs", worker+".log"))
			if len(b) > 0 {
				_ = os.WriteFile(filepath.Join(reports, strings.ReplaceAll(t.Name(), "/", "-")+"-supervised-"+worker+".log"), b, 0600)
			}
		}
	})
	control, err := node.New(node.Options{Mesh: nodeNetwork{n}, HubIdentity: f.identity.Public().(ed25519.PublicKey), Version: "0.1.0", Operator: u.Username, Workers: forwardedController{node.NewWorkerController(s), n.url}})
	if err != nil {
		t.Fatal(err)
	}
	client := connect(t, f.ctx, n.url, "mesh-node", "default")
	if err = control.Register(client); err != nil {
		t.Fatal(err)
	}
	return s
}
func (f *fixture) reload() {
	mustCall(f.t, f.admin, "default", "configuration::set", map[string]any{"id": "mesh", "value": f.config})
}
func (f *fixture) startIDE(n *testNode) string {
	t := f.t
	t.Helper()
	root := t.TempDir()
	mustCall(t, f.admin, "default", "configuration::ensure", map[string]any{"id": n.name + "__ide", "name": "IDE", "description": "Confined test IDE", "schema": map[string]any{}, "initial_value": map[string]any{"working_dir": root, "fs": map[string]any{"host_roots": []string{root}, "allow_unjailed": false}, "turns": map[string]any{"data_dir": filepath.Join(f.dir, "turns")}}})
	s := f.startNode(n, true)
	eventually(t, 30*time.Second, "supervised IDE registers", func() error {
		_, err := call(f.ctx, f.admin, f.ns, n.name+"::shell::exec", map[string]any{"command": "printf mesh-bootstrap", "cwd": root})
		if err != nil {
			return fmt.Errorf("%w; supervisor: %+v", err, s.Status())
		}
		return nil
	})
	return root
}

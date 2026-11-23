package hub

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/gateway"
)

func TestJoinCodeNeedsPublicURL(t *testing.T) {
	h := &Hub{config: DefaultConfig()}
	if _, err := h.JoinCode(context.Background(), struct{}{}); err == nil || err.Error() != "public_url is not set" {
		t.Fatalf("error = %v", err)
	}
}

func TestTakenIsEveryNamespaceAnotherWorkerUses(t *testing.T) {
	taken, err := workerNamespaces(json.RawMessage(`{"workers":[
		{"id":"1","name":"state","namespace":"default"},
		{"id":"2","name":"report","namespace":"analytics"},
		{"id":"3","name":"rbac-proxy:laptop::mesh-bridge","namespace":"laptop"},
		{"id":"4","name":"rbac-proxy:laptop::mesh-node","namespace":"laptop"},
		{"id":"5","name":"rbac-proxy:desk::mesh-bridge","namespace":"desk"},
		{"id":"6","name":"shell","namespace":"desk"},
		{"id":"7","name":"rbac-proxy:desk::mesh-bridge","namespace":"other"},
		{"id":"8","name":null,"namespace":"nameless"}]}`))
	if err != nil {
		t.Fatal(err)
	}
	// A machine's own sessions do not reserve its name; any other worker in a namespace does.
	for host, want := range map[string]string{"default": "default-2", "analytics": "analytics-2", "desk": "desk-2", "other": "other-2", "nameless": "nameless-2", "laptop": "laptop", "fresh": "fresh"} {
		if got := coordinator.AllocateName(host, func(s string) bool { return taken[s] }); got != want {
			t.Fatalf("%s -> %s, want %s", host, got, want)
		}
	}
}
func TestJoinCannotTakeTheHubNamespace(t *testing.T) {
	h, _, up := idleNodeCallHub(t) // the hub runs in `project`; the peer's hostname is laptop
	h.engine = &fakeEngine{}
	h.Namespace = "laptop"
	if err := h.refreshNames(t.Context()); err != nil {
		t.Fatal(err)
	}
	up()
	if nodes := h.Coordinator.Nodes(); len(nodes) != 1 || nodes[0].Name != "laptop-2" || !h.nameReserved("laptop") || !h.nameReserved("default") || h.nameReserved("laptop-2") {
		t.Fatalf("nodes %+v", nodes)
	}
}

// A hub project that starts under a connected machine's name must not share its namespace.
func TestANamespaceTakenUnderAConnectedMachineClosesIt(t *testing.T) {
	h, _, up := idleNodeCallHub(t) // the hub runs in `project`
	e := &fakeEngine{}
	h.engine, h.Tickets, h.Sessions = e, gateway.NewTickets(10*time.Second, time.Now), &Sessions{}
	up() // the peer joins as laptop
	auth := func() (AuthResult, error) {
		return h.Auth(t.Context(), AuthRequest{RegistrationNamespace: true, Headers: map[string]string{gateway.TicketHeader: h.Tickets.Mint("laptop")}})
	}
	session, err := auth()
	if err != nil {
		t.Fatal(err)
	}
	e.workers = []map[string]any{{"id": "1", "name": "rbac-proxy:laptop::mesh-bridge", "namespace": "laptop"}}
	if err := h.refreshNames(t.Context()); err != nil || h.nameReserved("laptop") {
		t.Fatalf("a machine's own session reserved its name: %v", err)
	}
	if _, ok := h.Sessions.Valid(session.Context.SessionKey); !ok {
		t.Fatal("a pass with nothing taken closed the machine")
	}
	e.workers = append(e.workers, map[string]any{"id": "2", "name": "state", "namespace": "laptop"})
	if err := h.refreshNames(t.Context()); err != nil || !h.nameReserved("laptop") {
		t.Fatalf("a worker in the machine's namespace went unnoticed: %v", err)
	}
	if _, ok := h.Sessions.Valid(session.Context.SessionKey); ok {
		t.Fatal("the machine kept its session")
	}
	if _, err := auth(); err == nil || !strings.Contains(err.Error(), "client.name") {
		t.Fatalf("the machine was admitted again: %v", err)
	}
}

func TestWorkerNamespacesRejectMalformedListing(t *testing.T) {
	for _, raw := range []string{`{}`, `{"workers":null}`, `{"workers":[{"id":"1","name":"state"}]}`} {
		if _, err := workerNamespaces(json.RawMessage(raw)); err == nil {
			t.Fatalf("accepted malformed listing %s", raw)
		}
	}
}

// Machines stay listed while the hub has no address, so removing one must work then.
func TestRemoveNodeNeedsNoPublicURL(t *testing.T) {
	c, err := coordinator.New(t.TempDir(), func(secret, addr string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	h := &Hub{config: DefaultConfig(), Coordinator: c}
	if _, err = h.RemoveNode(context.Background(), NodeRequest{Name: "ghost"}); err == nil || err.Error() != `no removable node "ghost"` {
		t.Fatalf("error = %v", err)
	}
}

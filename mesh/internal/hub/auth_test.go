package hub

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"reflect"
	"slices"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/gateway"
)

func authHub() *Hub {
	return &Hub{Namespace: "proj", config: DefaultConfig(), Tickets: gateway.NewTickets(10*time.Second, time.Now), Sessions: &Sessions{}}
}
func TestAuthGrants(t *testing.T) {
	h := authHub()
	h.defaults = GrantDefaults{Allow: []string{"shell::exec"}}
	ticket := h.Tickets.Mint("laptop")
	r := AuthRequest{Headers: map[string]string{gateway.TicketHeader: ticket}}
	got, err := h.Auth(context.Background(), r)
	if err != nil {
		t.Fatal(err)
	}
	if got.FunctionRegistrationPrefix != "laptop" {
		t.Fatalf("identity: %+v", got)
	}
	if !reflect.DeepEqual(got.Namespaces["default"], []string{"configuration::get", "configuration::ensure", "configuration::register", "configuration::set", "configuration::on-updated"}) || !reflect.DeepEqual(got.Namespaces["proj"], []string{"shell::exec"}) {
		t.Fatalf("namespaces: %+v", got.Namespaces)
	}

	wire, _ := json.Marshal(got)
	var auth map[string]json.RawMessage
	json.Unmarshal(wire, &auth)
	for _, removed := range []string{"allow_trigger_type_registration", "allowed_trigger_types"} {
		if _, ok := auth[removed]; ok {
			t.Fatalf("removed auth field %s", removed)
		}
	}
	if got.Context.MeshNode != "laptop" || len(got.Context.SessionKey) != 64 {
		t.Fatal("bad session identity")
	}
	if _, err := hex.DecodeString(got.Context.SessionKey); err != nil {
		t.Fatal(err)
	}
	if node, ok := h.Sessions.Valid(got.Context.SessionKey); !ok || node != "laptop" {
		t.Fatal("session not live")
	}
	if _, err = h.Auth(context.Background(), r); err == nil {
		t.Fatal("ticket replay accepted")
	}
	for _, r := range []AuthRequest{{}, {Headers: map[string]string{gateway.TicketHeader: "invalid"}}} {
		if _, err = h.Auth(context.Background(), r); err == nil {
			t.Fatal("missing/invalid ticket accepted")
		}
	}
	h.Namespace = "default"
	got, err = h.Auth(context.Background(), AuthRequest{Headers: map[string]string{gateway.TicketHeader: h.Tickets.Mint("laptop")}})
	if err != nil || len(got.Namespaces) != 1 || !slices.Contains(got.Namespaces["default"], "shell::exec") {
		t.Fatalf("merged grants: %+v %v", got, err)
	}
}

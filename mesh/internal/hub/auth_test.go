package hub

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"reflect"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/gateway"
)

func authHub() *Hub {
	return &Hub{Namespace: "proj", config: DefaultConfig(), Tickets: gateway.NewTickets(10*time.Second, time.Now), Sessions: &Sessions{}}
}
func TestAuthGrants(t *testing.T) {
	h := authHub()
	h.defaults = GrantDefaults{Allow: DefaultAllow()}
	ticket := h.Tickets.Mint("laptop")
	r := AuthRequest{RegistrationNamespace: true, Headers: map[string]string{gateway.TicketHeader: ticket}}
	got, err := h.Auth(context.Background(), r)
	if err != nil {
		t.Fatal(err)
	}
	if got.RegistrationNamespace != "laptop" {
		t.Fatalf("identity: %+v", got)
	}
	if !reflect.DeepEqual(got.Namespaces["default"], []string{"configuration::get", "configuration::ensure", "configuration::register", "configuration::set", "configuration::on-updated"}) || !reflect.DeepEqual(got.Namespaces["proj"], []string{"mesh::call", "mesh::node::changed"}) {
		t.Fatalf("namespaces: %+v", got.Namespaces)
	}
	if _, ok := got.Namespaces["laptop"]; ok {
		t.Fatal("a grant in the pin namespace")
	}

	wire, _ := json.Marshal(got)
	var auth map[string]json.RawMessage
	json.Unmarshal(wire, &auth)
	for _, removed := range []string{"allow_trigger_type_registration", "allowed_trigger_types", "function_registration_prefix"} {
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
	for _, r := range []AuthRequest{{RegistrationNamespace: true}, {RegistrationNamespace: true, Headers: map[string]string{gateway.TicketHeader: "invalid"}}} {
		if _, err = h.Auth(context.Background(), r); err == nil {
			t.Fatal("missing/invalid ticket accepted")
		}
	}
	h.Namespace = "default"
	got, err = h.Auth(context.Background(), AuthRequest{RegistrationNamespace: true, Headers: map[string]string{gateway.TicketHeader: h.Tickets.Mint("laptop")}})
	if err != nil || len(got.Namespaces) != 1 || !slices.Contains(got.Namespaces["default"], "mesh::call") {
		t.Fatalf("merged grants: %+v %v", got, err)
	}
}
func TestAuthRefusesAProxyThatDoesNotPin(t *testing.T) {
	h := authHub()
	ticket := h.Tickets.Mint("laptop")
	_, err := h.Auth(context.Background(), AuthRequest{Headers: map[string]string{gateway.TicketHeader: ticket}})
	if err == nil || !strings.Contains(err.Error(), "update rbac-proxy") {
		t.Fatalf("an auth request from a proxy that does not pin: %v", err)
	}
	// The ticket was not spent on the refusal.
	if _, err = h.Auth(context.Background(), AuthRequest{RegistrationNamespace: true, Headers: map[string]string{gateway.TicketHeader: ticket}}); err != nil {
		t.Fatal(err)
	}
}
func TestAuthRefusesAReservedMachineName(t *testing.T) {
	for _, ns := range []string{"proj", "default"} {
		h := authHub()
		h.Namespace = ns
		for _, name := range []string{"default", ns} {
			_, err := h.Auth(context.Background(), AuthRequest{RegistrationNamespace: true, Headers: map[string]string{gateway.TicketHeader: h.Tickets.Mint(name)}})
			if err == nil || !strings.Contains(err.Error(), "client.name") {
				t.Fatalf("hub in %s admitted a machine named %s: %v", ns, name, err)
			}
		}
	}
}

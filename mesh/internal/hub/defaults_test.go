package hub

import (
	"encoding/json"
	"reflect"
	"slices"
	"strings"
	"testing"

	"github.com/iii-hq/workers/mesh/internal/gateway"
)

func TestDefaultsExcludeUnconfinable(t *testing.T) {
	h := authHub()
	h.defaults = GrantDefaults{Allow: DefaultAllow()}
	h.config.Nodes["laptop"] = NodeConfig{Allow: []string{"audit::write"}}
	result, err := h.Auth(t.Context(), AuthRequest{Headers: map[string]string{gateway.TicketHeader: h.Tickets.Mint("laptop")}})
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(result.Namespaces["default"], []string{"configuration::get", "configuration::ensure", "configuration::register", "configuration::set", "configuration::on-updated"}) || !slices.Contains(result.Namespaces["proj"], "audit::write") {
		t.Fatalf("required owned configuration / operator grants missing: %+v", result.Namespaces)
	}
	if !reflect.DeepEqual(h.Config().Nodes["laptop"].Allow, []string{"audit::write"}) {
		t.Fatal("default grants changed operator configuration")
	}

	for _, target := range []string{"harness::hook::pre-trigger", "state::get", "stream::read", "queue::enqueue", "http::request", "cron::tick", "console:script"} {
		grants := slices.Clone(DefaultAllow())
		for _, functions := range result.Namespaces {
			grants = append(grants, functions...)
		}
		for _, grant := range grants {
			for _, prefix := range []string{"harness::hook::", "state::", "stream::", "queue::", "http::", "cron::", "console:"} {
				if strings.HasPrefix(grant, prefix) || grant == strings.TrimSuffix(prefix, "::") {
					t.Errorf("unconfined effective grant: %s", grant)
				}
			}
			if grant == target || strings.HasSuffix(grant, "*") && strings.HasPrefix(target, strings.TrimSuffix(grant, "*")) {
				t.Errorf("unconfined %s matches effective grant %s", target, grant)
			}
		}
		if _, err := h.ObserverHook(t.Context(), ObserverHookRequest{FunctionID: target, ResolvedFunctionID: target, Namespace: "proj", TargetFunctionID: "ide::on-change", ResolvedTargetFunctionID: "laptop::ide::on-change", TargetNamespace: "proj", Args: json.RawMessage(`{}`), Context: result.Context}); err == nil {
			t.Errorf("unconfined binding %s accepted", target)
		}
	}
}

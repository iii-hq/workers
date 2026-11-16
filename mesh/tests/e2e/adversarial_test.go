//go:build e2e

package e2e

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/netip"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	meshclient "github.com/iii-hq/workers/mesh/internal/client"
	"github.com/iii-hq/workers/mesh/internal/hub"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
)

func fakeWorker(f *fixture, n *testNode, name string) *iii.Client {
	c := connect(f.t, f.ctx, n.url, name, f.ns)
	if _, err := iii.RegisterFunctionTyped(c, "echo", func(context.Context, json.RawMessage) (map[string]string, error) {
		return map[string]string{"node": n.name}, nil
	}); err != nil {
		f.t.Fatal(err)
	}
	eventually(f.t, 5*time.Second, "fake echo registration", func() error {
		_, err := call(f.ctx, f.admin, f.ns, n.name+"::echo", map[string]any{"node": n.name})
		return err
	})
	return c
}
func refused(t *testing.T, c *iii.Client, ns, id string, p any, reason string) {
	t.Helper()
	out, err := call(t.Context(), c, ns, id, p)
	if err == nil || !strings.Contains(strings.ToLower(err.Error()), strings.ToLower(reason)) {
		t.Fatalf("%s: got %s, %v; want refusal containing %q", id, out, err, reason)
	}
}
func configEntry(id string, value any) map[string]any {
	return map[string]any{"id": id, "name": "test", "description": "test", "schema": map[string]any{}, "initial_value": value}
}
func TestE2ETemplatesRefused(t *testing.T) {
	f := newFixture(t, "default")
	n := f.enroll("evil")
	c := fakeWorker(f, n, "probe")
	for _, field := range []string{"value", "initial_value"} {
		p := configEntry("evil__probe", map[string]any{})
		p[field] = map[string]any{"secret": "${OPENAI_API_KEY}"}
		refused(t, c, "default", "configuration::ensure", p, "templates")
	}
	mustCall(t, f.admin, "default", "configuration::ensure", configEntry("evil__operator", map[string]any{"raw": "${OPENAI_API_KEY}"}))
	result := mustCall(t, c, "default", "configuration::get", map[string]any{"id": "evil__operator"})
	if !strings.Contains(string(result), "${OPENAI_API_KEY}") {
		t.Fatalf("expanded operator value: %s", result)
	}
	refused(t, c, "default", "configuration::get", map[string]any{"id": "mesh"}, "outside node scope")
}
func TestE2EMiddlewareNeedsSessionKey(t *testing.T) {
	f := newFixture(t, "default")
	refused(t, f.admin, "default", "mesh::middleware", map[string]any{"function_id": "configuration::get", "payload": map[string]any{"id": "mesh"}}, "invalid mesh session")
}
func TestE2ELoopbackWithoutTicket(t *testing.T) {
	f := newFixture(t, "default")
	ctx, cancel := context.WithTimeout(f.ctx, 3*time.Second)
	defer cancel()
	conn, _, err := websocket.Dial(ctx, f.proxyURL, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.CloseNow()
	_, message, err := conn.Read(ctx)
	if err != nil || !strings.Contains(string(message), `"type":"error"`) || !strings.Contains(string(message), "AUTH") {
		t.Fatalf("ticketless proxy did not reject: %s, %v", message, err)
	}
	_, _, err = conn.Read(ctx)
	if websocket.CloseStatus(err) == -1 {
		t.Fatalf("ticketless connection not closed: %v", err)
	}

}
func TestE2ENodeRegistersMeshIDGetsPrefix(t *testing.T) {
	f := newFixture(t, "default")
	n := f.enroll("evil")
	c := fakeWorker(f, n, "mesh")
	if _, err := iii.RegisterFunctionTyped(c, "mesh::status", func(context.Context, json.RawMessage) (map[string]bool, error) {
		return map[string]bool{"evil": true}, nil
	}); err != nil {
		t.Fatal(err)
	}
	eventually(t, 5*time.Second, "prefixed mesh registration", func() error {
		b, err := call(f.ctx, f.admin, "default", "evil::mesh::status", map[string]any{})
		if err == nil && !strings.Contains(string(b), `"evil":true`) {
			return fmt.Errorf("unexpected %s", b)
		}
		return err
	})
	original := mustCall(t, f.admin, "default", "mesh::status", map[string]any{})
	if strings.Contains(string(original), `"evil":true`) {
		t.Fatal("node replaced hub function")
	}
}
func (f *fixture) startFP() {
	startProcess(f.t, f.dir, "fp", binary(f.t, "MESH_TEST_FP_BIN"), nil, "--url", f.engineURL)
	eventually(f.t, 15*time.Second, "fp ready", func() error {
		_, err := call(f.ctx, f.admin, f.ns, "fp::batch", map[string]any{"calls": []any{map[string]any{"function": "mesh::status", "payload": map[string]any{}}}})
		return err
	})
}
func TestE2ENodeEndpointUnreachable(t *testing.T) {
	f := newFixture(t, "default")
	n := f.enroll("laptop")
	f.startNode(n, noWorkers{})
	f.startFP()
	eventually(t, 5*time.Second, "signed node endpoint registered", func() error {
		_, err := call(f.ctx, f.admin, "default", "laptop::mesh-node::workers::apply", map[string]any{})
		if err != nil && strings.Contains(err.Error(), "unsigned") {
			return nil
		}
		return fmt.Errorf("got %v", err)
	})
	refused(t, f.admin, "default", "laptop::mesh-node::workers::apply", map[string]any{}, "unsigned")
	for _, kind := range []string{"pipe", "batch"} {
		key := "through"
		if kind == "batch" {
			key = "calls"
		}
		b, err := call(f.ctx, f.admin, "default", "fp::"+kind, map[string]any{key: []any{map[string]any{"function": "laptop::mesh-node::workers::apply", "payload": map[string]any{}}}})
		rejection := fmt.Sprintf("%s %v", b, err)
		if !strings.Contains(rejection, "bus-internal functions are not supported") {
			t.Fatalf("%s did not refuse node endpoint: %s", kind, rejection)
		}

	}
	var envelope any
	eventually(t, 5*time.Second, "hub signed apply observed", func() error {
		for _, frame := range n.snapshot() {
			if frame.Direction == "hub-to-node" && frame.Data["type"] == "invokefunction" && frame.Data["function_id"] == "mesh-node::workers::apply" {
				payload, _ := frame.Data["data"].(map[string]any)
				if signature, ok := payload["sig"].(string); !ok || signature == "" {
					continue
				}
				envelope = payload
				return nil
			}
		}
		return fmt.Errorf("no signed apply")
	})
	refused(t, f.admin, "default", "laptop::mesh-node::workers::apply", envelope, "replayed")
}

func observe(t *testing.T, c *iii.Client, source iii.Source, target, id string, allowed bool) *iii.ObserverHandle {
	t.Helper()
	h, err := c.Observe(source, target, iii.ObserverOptions{ID: id})
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	err = h.Ready(ctx)
	if allowed {
		if err != nil {
			t.Fatalf("observe %s: %v", id, err)
		}
	} else {
		var rejection *iii.ObserveRejectionError
		if !errors.As(err, &rejection) || rejection.Code != "FORBIDDEN" {
			t.Fatalf("observer %s not refused: %v", id, err)
		}
	}
	return h
}

func TestE2EConditionCallbackRefused(t *testing.T) {
	f := newFixture(t, "project")
	n := f.enroll("evil")
	c := fakeWorker(f, n, "echo")
	var called atomic.Int32
	if _, err := iii.RegisterFunctionTyped(f.admin, "outside::callback", func(context.Context, json.RawMessage) (bool, error) { called.Add(1); return true, nil }); err != nil {
		t.Fatal(err)
	}
	// The bare spelling collides with a hub producer and must resolve to the node.
	if _, err := iii.RegisterFunctionTyped(f.admin, "hub::collision", func(_ context.Context, p json.RawMessage) (json.RawMessage, error) { return p, nil }); err != nil {
		t.Fatal(err)
	}
	mustCall(t, c, "default", "configuration::ensure", configEntry("evil__probe", map[string]any{}))
	delivered := make(chan json.RawMessage, 16)
	if _, err := iii.RegisterFunctionTyped(c, "on-change", func(_ context.Context, raw json.RawMessage) (bool, error) { delivered <- raw; return true, nil }); err != nil {
		t.Fatal(err)
	}
	owned := iii.Filters{"input.id": iii.Op{"$eq": "evil__probe"}, "input.event_type": iii.Op{"$eq": "configuration:updated"}}
	sources := []iii.Source{
		{FunctionID: "configuration::on-updated", Namespace: "default"},
		{FunctionID: "configuration::on-updated", Namespace: "default", Filters: iii.Filters{"input.id": iii.Op{"$eq": "mesh"}}},
		{FunctionID: "configuration::on-updated", Namespace: "default", Filters: iii.Filters{"input.id": iii.Op{"$eq": "laptop__probe"}}},
		{FunctionID: "configuration::on-updated", Namespace: "foreign", Filters: owned},
		{FunctionID: "configuration::on-updated", Namespace: "default", Filters: iii.Filters{"input.id": iii.Op{"$in": []string{"evil__probe", "mesh"}}}},
		{FunctionID: "configuration::on-updated", Namespace: "default", Filters: iii.Filters{"input.id": iii.Op{"$eq": true}}},
		{FunctionID: "configuration::on-updated", Namespace: "default", Filters: owned, Args: iii.Args{"condition_function_id": "outside::callback"}},
		{FunctionID: "configuration::on-updated", Namespace: "default", Filters: owned, Args: iii.Args{"condition_function_id": "echo"}},
		{FunctionID: "configuration::on-updated", Namespace: "default", Filters: owned, Args: iii.Args{"id": "mesh"}},
		{FunctionID: "echo", Namespace: "foreign"},
		{FunctionID: "laptop::echo", Namespace: f.ns},
		{FunctionID: "hub::collision", Namespace: f.ns},
	}
	for _, id := range []string{"state::on-changed", "queue::on-message", "http::on-request", "cron::on-tick", "harness::hook::on-pre-call", "ade::ui::asset"} {
		sources = append(sources, iii.Source{FunctionID: id, Namespace: f.ns})
	}
	for i, source := range sources {
		observe(t, c, source, "on-change", fmt.Sprintf("denied-%d", i), false)
	}
	if _, err := iii.RegisterFunctionTyped(f.admin, "echo", func(context.Context, json.RawMessage) (map[string]string, error) {
		return map[string]string{"node": "hub"}, nil
	}); err != nil {
		t.Fatal(err)
	}
	own := observe(t, c, iii.Source{FunctionID: "echo", Namespace: f.ns, Capture: []iii.Capture{iii.CaptureOutput}}, "on-change", "positive-own", true)
	mustCall(t, f.admin, f.ns, "echo", map[string]any{"secret": "foreign-producer"})
	select {
	case raw := <-delivered:
		t.Fatalf("bare-name observer leaked hub source: %s", raw)
	case <-time.After(150 * time.Millisecond):
	}

	mustCall(t, f.admin, f.ns, "evil::echo", map[string]any{})
	select {
	case raw := <-delivered:
		if !strings.Contains(string(raw), `"node":"evil"`) {
			t.Fatalf("own event: %s", raw)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("own event not delivered")
	}
	own.Dispose(f.ctx)
	cfg := observe(t, c, iii.Source{FunctionID: "configuration::on-updated", Namespace: "default", Filters: owned, Capture: []iii.Capture{iii.CaptureOutput}}, "on-change", "positive-config", true)
	defer cfg.Dispose(f.ctx)
	mustCall(t, c, "default", "configuration::set", map[string]any{"id": "evil__probe", "value": map[string]any{"changed": true}})
	select {
	case raw := <-delivered:
		if !strings.Contains(string(raw), `"id":"evil__probe"`) {
			t.Fatalf("config event: %s", raw)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("configuration event not delivered")
	}
	if called.Load() != 0 {
		t.Fatal("ungranted condition ran")
	}
}
func TestE2ENameSquatting(t *testing.T) {
	f := newFixture(t, "default")
	evil := f.enroll("evil")
	fakeWorker(f, evil, "ide")
	before := mustCall(t, f.admin, "default", "engine::workers::list", map[string]any{})
	if !strings.Contains(string(before), "rbac-proxy:evil::ide") {
		t.Fatalf("unconfined evil worker name: %s", before)
	}
	mac := f.enroll("mac")
	fakeWorker(f, mac, "ide")
	workers := mustCall(t, f.admin, "default", "engine::workers::list", map[string]any{})
	for _, name := range []string{"rbac-proxy:evil::ide", "rbac-proxy:mac::ide"} {
		if !strings.Contains(string(workers), name) {
			t.Fatalf("missing %s: %s", name, workers)
		}
	}
	for _, name := range []string{"evil", "mac"} {
		b := mustCall(t, f.admin, "default", name+"::echo", map[string]any{"probe": "same-payload"})
		var reply struct{ Node string }
		if err := json.Unmarshal(b, &reply); err != nil || reply.Node != name {
			t.Fatalf("displaced %s: %s", name, b)
		}
	}
}

func TestE2ETwoNodesSameWorkerName(t *testing.T) {
	f := newFixture(t, "default")
	for _, name := range []string{"alpha", "beta"} {
		n := f.enroll(name)
		c := fakeWorker(f, n, "echo")
		received := make(chan json.RawMessage, 4)
		if _, err := iii.RegisterFunctionTyped(c, "received", func(_ context.Context, p struct {
			Output json.RawMessage `json:"output"`
		}) (bool, error) {
			received <- p.Output
			return true, nil
		}); err != nil {
			t.Fatal(err)
		}
		h := observe(t, c, iii.Source{FunctionID: "echo", Namespace: f.ns, Capture: []iii.Capture{iii.CaptureOutput}}, "received", "", true)
		mustCall(t, f.admin, f.ns, name+"::echo", map[string]any{"probe": "same-payload"})
		select {
		case raw := <-received:
			var reply struct{ Node string }
			if json.Unmarshal(raw, &reply) != nil || reply.Node != name {
				t.Fatalf("observer escaped node: %s", raw)
			}
		case <-time.After(5 * time.Second):
			t.Fatal("own event never reached fake worker")
		}
		defer h.Dispose(f.ctx)
	}
	for _, name := range []string{"alpha", "beta"} {
		out := mustCall(t, f.admin, "default", name+"::echo", map[string]any{"probe": "same-payload"})
		var reply struct{ Node string }
		if err := json.Unmarshal(out, &reply); err != nil || reply.Node != name {
			t.Fatalf("echo displaced: %s", out)
		}
	}
	workers := mustCall(t, f.admin, "default", "engine::workers::list", map[string]any{})
	for _, name := range []string{"alpha", "beta"} {
		if !strings.Contains(string(workers), "rbac-proxy:"+name+"::echo") {
			t.Fatalf("missing %s worker: %s", name, workers)
		}
	}
}

// The hub shares probe and one internal function by id; beta's bridge shares an
// echo; alpha runs a real bridge whose local project is a second namespace of the
// same engine, and shares one internal function by id too.
func TestE2EMeshSharing(t *testing.T) {
	f := newFixture(t, "project")
	answer := func(c *iii.Client, id, node string) {
		t.Helper()
		if _, err := iii.RegisterFunctionTyped(c, id, func(context.Context, json.RawMessage) (map[string]string, error) {
			return map[string]string{"node": node}, nil
		}); err != nil {
			t.Fatal(err)
		}
	}
	answerInternal := func(c *iii.Client, id, node string) {
		t.Helper()
		if _, err := c.RegisterFunction(id, func(context.Context, json.RawMessage) (any, error) {
			return map[string]string{"node": node}, nil
		}, iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true}`)}); err != nil {
			t.Fatal(err)
		}
	}
	share := func(workers ...string) {
		cfg := f.config
		cfg.Share = workers
		mustCall(t, f.admin, "default", "configuration::set", map[string]any{"id": "mesh", "value": cfg})
	}
	hubProbe := connect(t, f.ctx, f.engineURL, "probe", f.ns)
	answer(hubProbe, "probe::hello", "hub")
	answerInternal(hubProbe, "probe::cited", "hub")
	answerInternal(hubProbe, "probe::uncited", "hub")
	share("probe", "probe::cited")

	beta := f.enroll("beta")
	f.startNode(beta, noWorkers{})
	answer(connect(t, f.ctx, beta.url, "mesh-bridge", f.ns), "probe::echo", "beta")

	alpha := f.enroll("alpha")
	alphaProbe := connect(t, f.ctx, f.engineURL, "probe", "alpha-local")
	answer(alphaProbe, "probe::echo", "alpha")
	answerInternal(alphaProbe, "probe::secret", "alpha")
	answerInternal(alphaProbe, "probe::private", "alpha")
	bridge := meshclient.NewBridge(connect(t, f.ctx, f.engineURL, "mesh", "alpha-local"), "alpha-local")
	t.Cleanup(bridge.Close)
	if err := bridge.Sync(f.ctx, []string{"probe", "probe::secret"}); err != nil {
		t.Fatal(err)
	}
	f.startNode(alpha, bridge)

	listedAll := func(internal bool) (map[string]meshclient.Row, error) {
		out, err := call(f.ctx, f.admin, "default", "engine::functions::list", map[string]any{"include_internal": internal})
		var list struct{ Functions []meshclient.Row }
		if err == nil {
			err = json.Unmarshal(out, &list)
		}
		rows := map[string]meshclient.Row{}
		for _, r := range list.Functions {
			rows[r.Namespace+" "+r.FunctionID] = r
		}
		return rows, err
	}
	listed := func() (map[string]meshclient.Row, error) { return listedAll(false) }
	eventually(t, 30*time.Second, "alpha mirrors the hub's and beta's functions", func() error {
		rows, err := listed()
		if err != nil {
			return err
		}
		if _, ok := rows["alpha-local hub::probe::hello"]; !ok {
			return errors.New("hub::probe::hello missing")
		}
		// The function the hub shares by id is mirrored internal, as on the hub: out of the default listing.
		if _, ok := rows["alpha-local hub::probe::cited"]; ok {
			return errors.New("hub::probe::cited mirrored without internal metadata")
		}
		all, err := listedAll(true)
		if err != nil {
			return err
		}
		if r, ok := all["alpha-local hub::probe::cited"]; !ok || !meshclient.Internal(r.Metadata) {
			return fmt.Errorf("hub::probe::cited mirror %+v", r)
		}
		if _, ok := all["alpha-local hub::probe::uncited"]; ok {
			return errors.New("an uncited internal function mirrored")
		}
		if _, ok := rows["alpha-local peer::beta::probe::echo"]; !ok {
			return errors.New("peer::beta::probe::echo missing")
		}
		// The bridge rows carry the worker name that proves their owner.
		if w := rows["project beta::probe::echo"].WorkerName; w != "rbac-proxy:beta::mesh-bridge" {
			return fmt.Errorf("beta's echo listed under %q", w)
		}
		return nil
	})
	// Every machine's functions are in the catalog alpha holds.
	var last string // the last catalog sent, as JSON
	pushes := func() []string {
		revs := []string{}
		for _, frame := range alpha.snapshot() {
			if frame.Direction == "hub-to-node" && frame.Data["type"] == "invokefunction" && frame.Data["function_id"] == "mesh-node::catalog::apply" {
				envelope, _ := frame.Data["data"].(map[string]any)
				payload, _ := envelope["payload"].(map[string]any)
				rev, _ := payload["rev"].(string)
				revs, last = append(revs, rev), mustJSON(payload)
			}
		}
		return revs
	}
	eventually(t, 20*time.Second, "the catalog with alpha's own echo and cited secret sent", func() error {
		pushes()
		var c node.Catalog
		_ = json.Unmarshal([]byte(last), &c)
		internal := map[string]bool{}
		for _, e := range c.Functions {
			internal[e.ID] = e.Internal
		}
		if e, ok := internal["alpha::probe::echo"]; !ok || e || !internal["alpha::probe::secret"] || !internal["probe::cited"] {
			return fmt.Errorf("last catalog %s", last)
		}
		if _, ok := internal["alpha::probe::private"]; ok {
			return errors.New("an uncited internal function of alpha in the catalog")
		}
		return nil
	})
	for id, node := range map[string]string{"hub::probe::hello": "hub", "peer::beta::probe::echo": "beta", "hub::probe::cited": "hub"} {
		var reply struct{ Node string }
		if err := json.Unmarshal(mustCall(t, f.admin, "alpha-local", id, map[string]any{}), &reply); err != nil || reply.Node != node {
			t.Fatalf("%s answered %+v: %v", id, reply, err)
		}
	}
	var status hub.StatusResult
	if err := json.Unmarshal(mustCall(t, f.admin, f.ns, "mesh::status", map[string]any{}), &status); err != nil || mustJSON(status.Shared) != `[{"worker":"probe","state":"shared","functions":["hub::probe::hello"]},{"worker":"probe::cited","state":"shared","functions":["hub::probe::cited"]}]` {
		t.Fatalf("hub status shared %s: %v", mustJSON(status.Shared), err)
	}

	// What a raw session of alpha can and cannot reach through mesh::call.
	raw := connect(t, f.ctx, alpha.url, "raw", f.ns)
	meshCall := func(owner, id string, payload any) map[string]any {
		return map[string]any{"owner": owner, "function_id": id, "payload": payload}
	}
	if out := mustCall(t, raw, f.ns, "mesh::call", meshCall("beta", "beta::probe::echo", map[string]any{})); !strings.Contains(string(out), `"node":"beta"`) {
		t.Fatalf("beta's echo for alpha: %s", out)
	}
	for _, c := range [][2]string{{"hub", "other::hidden"}, {"hub", "mesh::join::code"}, {"hub", "engine::functions::list"}, {"beta", "beta::mesh-node::status"}, {"alpha", "alpha::probe::echo"}, {"hub", "beta::probe::echo"}, {"hub", "probe::uncited"}} {
		refused(t, raw, f.ns, "mesh::call", meshCall(c[0], c[1], map[string]any{}), "forbidden")
	}
	// What alpha shares by id reaches beta; its uncited internal function does not.
	betaRaw := connect(t, f.ctx, beta.url, "raw", f.ns)
	if out := mustCall(t, betaRaw, f.ns, "mesh::call", meshCall("alpha", "alpha::probe::secret", map[string]any{})); !strings.Contains(string(out), `"node":"alpha"`) {
		t.Fatalf("alpha's secret for beta: %s", out)
	}
	refused(t, betaRaw, f.ns, "mesh::call", meshCall("alpha", "alpha::probe::private", map[string]any{}), "forbidden")
	enqueued, _ := json.Marshal(meshCall("beta", "beta::probe::echo", map[string]any{}))
	if _, err := raw.Call(f.ctx, iii.CallRequest{FunctionID: "mesh::call", Namespace: f.ns, Payload: enqueued, Action: iii.EnqueueAction("mesh"), Timeout: time.Second}); err == nil || !strings.Contains(strings.ToLower(err.Error()), "forbidden") {
		t.Fatalf("enqueued mesh::call: %v", err)
	}
	refused(t, raw, f.ns, "mesh::call", meshCall("beta", "beta::probe::echo", map[string]any{"reader": map[string]any{"channel_id": "c", "access_key": "k", "direction": "read"}}), "channels_unsupported")

	// A catalog is sent only to a node holding another revision.
	sent := pushes()
	for i := 1; i < len(sent); i++ {
		if sent[i] == sent[i-1] {
			t.Fatalf("the same catalog sent twice: %v", sent)
		}
	}
	answer(connect(t, f.ctx, f.engineURL, "other", f.ns), "other::hidden", "hub") // re-reads the catalog and every node
	time.Sleep(2 * time.Second)
	if again := pushes(); len(again) != len(sent) {
		t.Fatalf("an unchanged catalog sent again: %v after %v", again, sent)
	}
	rows, err := listed()
	for key := range rows {
		if strings.HasPrefix(key, "alpha-local peer::alpha::") || err != nil {
			t.Fatalf("own function mirrored: %s %v", key, err)
		}
	}

	share()
	eventually(t, 20*time.Second, "an unshared hub function refused", func() error {
		if _, err := call(f.ctx, raw, f.ns, "mesh::call", meshCall("hub", "probe::hello", map[string]any{})); err == nil {
			return errors.New("still callable")
		}
		return nil
	})
	eventually(t, 20*time.Second, "the mirror removed", func() error {
		rows, err := listed()
		if _, ok := rows["alpha-local hub::probe::hello"]; err == nil && ok {
			return errors.New("hub::probe::hello still mirrored")
		}
		return err
	})
	if revs := pushes(); len(revs) != len(sent)+1 || revs[len(revs)-1] == sent[len(sent)-1] {
		t.Fatalf("pushes after the revision moved: %v, before %v", revs, sent)
	}

	mustCall(t, f.admin, f.ns, "mesh::nodes::remove", map[string]any{"name": "alpha"})
	eventually(t, 20*time.Second, "a removed machine refused", func() error {
		if _, err := call(f.ctx, raw, f.ns, "mesh::call", meshCall("beta", "beta::probe::echo", map[string]any{})); err == nil {
			return errors.New("still callable")
		}
		return nil
	})
}

// Security: over the mesh a joined machine opens the hub node's gateway port and nothing else, the backend's peer API included.
func TestE2EHubTakesOnlyItsGateway(t *testing.T) {
	f := newFixture(t, "default")
	n := f.enrollMesh("probe")
	gateway, err := netip.ParseAddrPort(f.hubAddress)
	if err != nil {
		t.Fatal(err)
	}
	to := gateway.Addr()
	eventually(t, 10*time.Second, "the gateway port to open", func() error { return dial(n.mesh, to, gateway.Port()) })
	for _, port := range []uint16{f.peerAPIPort(t, "hub", to), meshnet.PeerPort} {
		if err := dial(n.mesh, to, port); err == nil {
			t.Fatalf("the machine opened port %d on the hub node", port)
		}
	}
}

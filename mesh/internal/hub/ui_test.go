package hub

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
)

func TestUIContentServesEmbeddedAssets(t *testing.T) {
	for _, asset := range []struct{ path, kind, marker string }{
		{"mesh/page.js", "text/javascript; charset=utf-8", "export"},
		{"mesh/styles.css", "text/css; charset=utf-8", "[data-iii-ui=mesh]"},
	} {
		got, err := uiContent(t.Context(), UIRequest{Path: asset.path})
		if err != nil || got.ContentType != asset.kind || !strings.Contains(got.Content, asset.marker) {
			t.Fatalf("%s: type=%q bytes=%d err=%v", asset.path, got.ContentType, len(got.Content), err)
		}
	}
	for _, path := range []string{"page.js", "mesh/../page.js", "other/page.js", "mesh/missing.css"} {
		if _, err := uiContent(t.Context(), UIRequest{Path: path}); err == nil {
			t.Fatalf("served unknown path %q", path)
		}
	}
}

func TestConfigurationIDHonoursConfigName(t *testing.T) {
	t.Setenv("III_CONFIG_NAME", "mesh-lab")
	t.Setenv("III_NAMESPACE", "project")
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	defer cancel()
	result := make(chan json.RawMessage, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		for {
			_, b, err := conn.Read(ctx)
			if err != nil {
				return
			}
			var frame map[string]json.RawMessage
			if json.Unmarshal(b, &frame) != nil {
				return
			}
			if string(frame["type"]) == `"registerfunction"` && string(frame["id"]) == `"mesh::configuration-id"` {
				b = []byte(`{"type":"invokefunction","invocation_id":"00000000-0000-4000-8000-000000000001","function_id":"mesh::configuration-id","data":{}}`)
				if conn.Write(ctx, websocket.MessageText, b) != nil {
					return
				}
			}
			if string(frame["type"]) == `"invocationresult"` {
				result <- frame["result"]
				return
			}
		}
	}))
	defer server.Close()
	done := make(chan error, 1)
	go func() {
		done <- Run(ctx, Options{URL: "ws" + strings.TrimPrefix(server.URL, "http"), LocalConfig: true})
	}()
	select {
	case got := <-result:
		if string(got) != `{"id":"mesh-lab"}` {
			t.Fatalf("identity=%s", got)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	cancel()
	if err := <-done; err != nil {
		t.Fatal(err)
	}
}

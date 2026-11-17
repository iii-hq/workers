package relay

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"tailscale.com/derp"
	"tailscale.com/derp/derphttp"
	"tailscale.com/net/netmon"
	"tailscale.com/tailcfg"
	"tailscale.com/types/key"
)

func TestRelayAdmitsOnlyKnownKeys(t *testing.T) {
	known := key.NewNode().Public()
	h := AdmitHandler(func(k key.NodePublic) bool { return k == known })
	for _, tc := range []struct {
		key     key.NodePublic
		addr    string
		allowed bool
		status  int
	}{{known, "127.0.0.1:1234", true, 200}, {key.NewNode().Public(), "[::1]:1234", false, 200}, {known, "192.0.2.1:1234", false, 403}} {
		b, _ := json.Marshal(tailcfg.DERPAdmitClientRequest{NodePublic: tc.key})
		r := httptest.NewRequest("POST", "/verify", bytes.NewReader(b))
		r.RemoteAddr = tc.addr
		w := httptest.NewRecorder()
		h.ServeHTTP(w, r)
		if w.Code != tc.status {
			t.Fatal(w.Code)
		}
		if w.Code == 200 {
			var resp tailcfg.DERPAdmitClientResponse
			if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil || resp.Allow != tc.allowed {
				t.Fatalf("response %v %v", resp, err)
			}
		}
	}
}
func TestRelayHandshakeFailsClosed(t *testing.T) {
	good := key.NewNode()
	admit := httptest.NewServer(AdmitHandler(func(k key.NodePublic) bool { return k == good.Public() }))
	defer admit.Close()
	unavailable := httptest.NewServer(http.NotFoundHandler())
	unavailable.Close()
	for _, tc := range []struct {
		name, url string
		client    key.NodePrivate
		allow     bool
	}{{"known", admit.URL, good, true}, {"unknown", admit.URL, key.NewNode(), false}, {"empty", "", good, false}, {"unavailable", unavailable.URL, good, false}, {"nonloopback", "https://invalid.example/", good, false}} {
		t.Run(tc.name, func(t *testing.T) {
			relay := New(tc.url)
			defer relay.Close()
			mux := http.NewServeMux()
			Handle(mux, relay)
			s := httptest.NewServer(mux)
			defer s.Close()
			client, err := derphttp.NewClient(tc.client, s.URL+"/derp", t.Logf, netmon.NewStatic())
			if err != nil {
				t.Fatal(err)
			}
			defer client.Close()
			ctx, cancel := context.WithTimeout(t.Context(), 3*time.Second)
			defer cancel()
			if err := client.Connect(ctx); err != nil && tc.allow {
				t.Fatal(err)
			}
			done := make(chan error, 1)
			go func() {
				m, err := client.Recv()
				if err == nil {
					if _, ok := m.(derp.ServerInfoMessage); !ok {
						t.Errorf("unexpected %T", m)
					}
				}
				done <- err
			}()
			select {
			case err := <-done:
				if (err == nil) != tc.allow {
					t.Fatalf("admit=%v error=%v", tc.allow, err)
				}
			case <-ctx.Done():
				t.Fatal("admission timed out")
			}
		})
	}
}

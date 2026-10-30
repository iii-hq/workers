package registry

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestResolvePicksNamedEntry(t *testing.T) {
	digest := strings.Repeat("aB", 32)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost || r.URL.Path != "/resolve" {
			t.Errorf("request = %s %s, want POST /resolve", r.Method, r.URL.Path)
		}
		if got := r.Header.Get("Content-Type"); got != "application/json" {
			t.Errorf("Content-Type = %q, want application/json", got)
		}
		var request map[string]string
		if err := json.NewDecoder(r.Body).Decode(&request); err != nil {
			t.Errorf("decode request: %v", err)
		}
		if len(request) != 3 || request["worker"] != "ide" || request["version"] != "^0.12" || request["target"] != "x86_64-unknown-linux-gnu" {
			t.Errorf("request = %v, want worker ide, version ^0.12, target x86_64-unknown-linux-gnu", request)
		}
		fmt.Fprintf(w, `{"graph":[
			{"name":"configuration","version":"1.0.0","type":"binary","binaries":{"x86_64-unknown-linux-gnu":{"url":"http://ignored.invalid/configuration","sha256":"ignored"}}},
			{"name":"ide","version":"0.12.36","type":"binary","binaries":{"x86_64-unknown-linux-gnu":{"url":"https://artifacts.example/ide","sha256":%q}}}
		],"edges":[{"from":"ide","to":"configuration"}]}`, digest)
	}))
	defer server.Close()

	got, err := Resolve(context.Background(), server.Client(), server.URL+"/", "ide", "^0.12", "x86_64-unknown-linux-gnu")
	if err != nil {
		t.Fatal(err)
	}
	want := Artifact{Version: "0.12.36", URL: "https://artifacts.example/ide", SHA256: digest}
	if got != want {
		t.Fatalf("artifact = %+v, want %+v", got, want)
	}
}

func TestResolveRejects(t *testing.T) {
	digest := strings.Repeat("a", 64)
	for _, tt := range []struct {
		name, kind, target, artifactURL, digest string
		want                                    error
	}{
		{"http", "binary", "target", "http://artifacts.example/ide", digest, ErrInsecureURL},
		{"relative URL", "binary", "target", "/ide", digest, ErrInsecureURL},
		{"hostless URL", "binary", "target", "https:///ide", digest, ErrInsecureURL},
		{"empty hostname", "binary", "target", "https://:443/ide", digest, ErrInsecureURL},
		{"malformed URL", "binary", "target", "https://artifacts.example/%zz", digest, ErrInsecureURL},
		{"short digest", "binary", "target", "https://artifacts.example/ide", digest[:63], ErrBadDigest},
		{"nonhex digest", "binary", "target", "https://artifacts.example/ide", strings.Repeat("g", 64), ErrBadDigest},
		{"missing digest", "binary", "target", "https://artifacts.example/ide", "", ErrBadDigest},
		{"image", "image", "target", "https://artifacts.example/ide", digest, ErrNotBinary},
		{"missing target", "binary", "other", "https://artifacts.example/ide", digest, ErrNoTarget},
	} {
		t.Run(tt.name, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				fmt.Fprintf(w, `{"graph":[{"name":"ide","version":"0.12.36","type":%q,"binaries":{%q:{"url":%q,"sha256":%q}}}]}`, tt.kind, tt.target, tt.artifactURL, tt.digest)
			}))
			defer server.Close()
			got, err := Resolve(context.Background(), server.Client(), server.URL, "ide", "^0.12", "target")
			if !errors.Is(err, tt.want) {
				t.Fatalf("error = %v, want %v", err, tt.want)
			}
			if got != (Artifact{}) {
				t.Fatalf("artifact on error = %+v, want zero value", got)
			}
		})
	}
}

func TestResolveRejectsMalformedGraph(t *testing.T) {
	digest := strings.Repeat("b", 64)
	for _, body := range []string{
		`{`,
		`{} tail`,
		`{} {}`,
		`{}`,
		`{"graph":[]}`,
		`{"graph":[null]}`,
		fmt.Sprintf(`{"graph":[{"name":"configuration","version":"1","type":"binary","binaries":{"target":{"url":"https://artifacts.example/configuration","sha256":%q}}}]}`, digest),
		fmt.Sprintf(`{"graph":[{"name":"ide","type":"binary","binaries":{"target":{"url":"https://artifacts.example/ide","sha256":%q}}}]}`, digest),
		`{"graph":[{"name":"ide","version":"1"}]}`,
		`{"graph":[{"name":"ide","version":"1","type":"binary","binaries":{}}]}`,
		`{"graph":[{"name":"ide","version":"1","type":"binary","binaries":{"target":null}}]}`,
		fmt.Sprintf(`{"graph":[{"name":"ide","version":"1","type":"binary","binaries":{"target":{"url":"https://artifacts.example/ide-1","sha256":%q}}},{"name":"ide","version":"2","type":"binary","binaries":{"target":{"url":"https://artifacts.example/ide-2","sha256":%q}}}]}`, digest, digest),
	} {
		t.Run(body, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				fmt.Fprint(w, body)
			}))
			defer server.Close()
			if got, err := Resolve(context.Background(), nil, server.URL, "ide", "*", "target"); err == nil || got != (Artifact{}) {
				t.Fatalf("Resolve = %+v, %v; want zero artifact and error", got, err)
			}
		})
	}
}

func TestResolveHTTPError(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
		fmt.Fprint(w, `{"graph":[]}`)
	}))
	defer server.Close()
	if _, err := Resolve(context.Background(), server.Client(), server.URL, "ide", "*", "target"); err == nil || !strings.Contains(err.Error(), "503") {
		t.Fatalf("error = %v, want HTTP 503 error", err)
	}
}

func TestResolveCanceledContext(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t.Error("canceled request reached server")
	}))
	defer server.Close()
	if _, err := Resolve(ctx, server.Client(), server.URL, "ide", "*", "target"); !errors.Is(err, context.Canceled) {
		t.Fatalf("error = %v, want context.Canceled", err)
	}
}

func TestTargetFor(t *testing.T) {
	for _, tt := range []struct {
		goos, goarch string
		musl         bool
		want         string
	}{
		{"linux", "amd64", false, "x86_64-unknown-linux-gnu"},
		{"linux", "amd64", true, "x86_64-unknown-linux-musl"},
		{"linux", "arm64", false, "aarch64-unknown-linux-gnu"},
		{"linux", "arm64", true, ""},
		{"darwin", "amd64", false, "x86_64-apple-darwin"},
		{"darwin", "arm64", false, "aarch64-apple-darwin"},
		{"windows", "amd64", false, "x86_64-pc-windows-msvc"},
		{"windows", "arm64", false, "aarch64-pc-windows-msvc"},
		{"darwin", "arm64", true, "aarch64-apple-darwin"},
		{"freebsd", "amd64", false, ""},
		{"linux", "386", false, ""},
	} {
		t.Run(fmt.Sprintf("%s/%s/musl=%t", tt.goos, tt.goarch, tt.musl), func(t *testing.T) {
			if got := TargetFor(tt.goos, tt.goarch, tt.musl); got != tt.want {
				t.Fatalf("target = %q, want %q", got, tt.want)
			}
		})
	}
}

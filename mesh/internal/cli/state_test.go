package cli

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"os"
	"path/filepath"
	"testing"

	"github.com/iii-hq/workers/mesh/internal/join"
	"tailscale.com/types/key"
)

func testState(t *testing.T) State {
	t.Helper()
	pub, private, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	control := key.NewMachine().Public()
	sig := ed25519.Sign(private, []byte("iii mesh control "+control.String()))
	secret := make([]byte, 32)
	rand.Read(secret)
	return State{PublicURL: "https://hub.example", Secret: hex.EncodeToString(secret), Identity: join.Identity{Identity: pub, Control: control, Signature: sig}, Operator: "alice", RequestedName: "laptop", Port: 41643}
}
func TestStatePrivateAndAuthenticated(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "state")
	s := testState(t)
	if err := saveState(dir, s); err != nil {
		t.Fatal(err)
	}
	got, err := loadState(dir)
	if err != nil {
		t.Fatal(err)
	}
	if got.Operator != "alice" || got.Port != 41643 || got.Secret != s.Secret {
		t.Fatal("state changed")
	}
	for path, want := range map[string]os.FileMode{dir: 0700, filepath.Join(dir, "join.json"): 0600} {
		st, err := os.Stat(path)
		if err != nil || st.Mode().Perm() != want {
			t.Fatalf("mode %s: %v %v", path, st, err)
		}
	}
	s.Identity.Signature[0] ^= 1
	if err := saveState(dir, s); err == nil {
		t.Fatal("stored unsigned control")
	}
}
func TestStateRefusesSymlink(t *testing.T) {
	dir := t.TempDir()
	real := filepath.Join(dir, "real")
	os.Mkdir(real, 0700)
	link := filepath.Join(dir, "link")
	os.Symlink(real, link)
	if err := saveState(link, testState(t)); err == nil {
		t.Fatal("followed state symlink")
	}
}

func TestStateKeepsCanonicalOperatorIdentity(t *testing.T) {
	s := testState(t)
	s.OperatorID = "S-1-5-21-123-456-789-1001"
	dir := filepath.Join(t.TempDir(), "state")
	if err := saveState(dir, s); err != nil {
		t.Fatal(err)
	}
	got, err := loadState(dir)
	if err != nil {
		t.Fatal(err)
	}
	if got.Operator != "alice" || got.operatorAccount() != s.OperatorID {
		t.Fatal("lost original stable operator identity")
	}
}

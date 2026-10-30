package meshnet

import (
	"net/netip"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestHostsBlockWritesOnlyInternalNames(t *testing.T) {
	for _, nl := range []string{"\n", "\r\n"} {
		t.Run(strings.ReplaceAll(nl, "\r", "CR"), func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "hosts")
			before := "# keep spacing  \t" + nl + "127.0.0.1 localhost" + nl
			after := "# keep this too" + nl
			old := before + "# iii mesh begin" + nl + "fd7a::9 stale.iii.internal" + nl + "# iii mesh end" + nl + after
			if err := os.WriteFile(path, []byte(old), 0640); err != nil {
				t.Fatal(err)
			}
			for _, names := range []map[string]netip.Addr{{"laptop": netip.MustParseAddr("fd7a::2")}, {"hub": netip.MustParseAddr("fd7a::1")}, nil} {
				if err := WriteHostsBlock(path, names); err != nil {
					t.Fatal(err)
				}
				data, err := os.ReadFile(path)
				if err != nil {
					t.Fatal(err)
				}
				want := before + after
				if len(names) > 0 {
					for name, addr := range names {
						want = before + "# iii mesh begin" + nl + addr.String() + " " + name + ".iii.internal" + nl + "# iii mesh end" + nl + after
					}
				}
				if string(data) != want {
					t.Fatalf("hosts = %q, want %q", data, want)
				}
			}
		})
	}
}

func TestHostsBlockRejectsInvalidNamesWithoutChangingFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "hosts")
	original := []byte("127.0.0.1 localhost\n")
	for _, name := range []string{"bad\ninjected", "laptop.elsewhere", "-bad"} {
		if err := os.WriteFile(path, original, 0644); err != nil {
			t.Fatal(err)
		}
		if err := WriteHostsBlock(path, map[string]netip.Addr{name: netip.MustParseAddr("fd7a::2")}); err == nil {
			t.Fatalf("accepted %q", name)
		}
		b, _ := os.ReadFile(path)
		if string(b) != string(original) {
			t.Fatal("changed hosts on invalid name")
		}
	}
}

func TestHostsBlockWindowsCRLF(t *testing.T) {
	for _, sequence := range []string{"begin begin end", "end begin end", "begin end begin", "begin end end", "begin end begin end", "begin", "end", "end begin"} {
		t.Run(sequence, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "hosts")
			content := "# keep\r\n"
			for _, marker := range strings.Fields(sequence) {
				content += "# iii mesh " + marker + "\r\n"
			}
			content += "127.0.0.1 localhost\r\n"
			if err := os.WriteFile(path, []byte(content), 0600); err != nil {
				t.Fatal(err)
			}
			if err := WriteHostsBlock(path, map[string]netip.Addr{"hub": netip.MustParseAddr("fd42::1")}); err == nil {
				t.Fatal("accepted malformed marker sequence")
			}
			got, _ := os.ReadFile(path)
			if string(got) != content {
				t.Fatal("modified malformed file")
			}
		})
	}
	path := filepath.Join(t.TempDir(), "hosts")
	before, after := "# keep\t\r\n", "# tail without newline"
	os.WriteFile(path, []byte(before+begin+"\r\nstale\r\n"+end+"\r\n"+after), 0600)
	if err := WriteHostsBlock(path, map[string]netip.Addr{"hub": netip.MustParseAddr("fd42::1")}); err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(path)
	want := before + begin + "\r\nfd42::1 hub.iii.internal\r\n" + end + "\r\n" + after
	if string(got) != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

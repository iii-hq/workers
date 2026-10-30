package service

import (
	"archive/zip"
	"io"
	"os"
	"testing"
)

func TestVerifyWintun(t *testing.T) {
	if err := verifyWintun([]byte{1}); err == nil {
		t.Fatal("accepted invalid DLL")
	}
	if err := verifyWintunArchitecture([]byte{1}, "386"); err == nil {
		t.Fatal("accepted unsupported architecture")
	}
}

func TestVerifyOfficialWintun(t *testing.T) {
	path := os.Getenv("WINTUN_TEST_ARCHIVE")
	if path == "" {
		t.Skip("set WINTUN_TEST_ARCHIVE to the official 0.14.1 archive for offline provenance verification")
	}
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := verifyWintunArchive(b); err != nil {
		t.Fatal(err)
	}
	z, err := zip.OpenReader(path)
	if err != nil {
		t.Fatal(err)
	}
	defer z.Close()
	for _, arch := range []string{"amd64", "arm64"} {
		f, err := z.Open("wintun/bin/" + arch + "/wintun.dll")
		if err != nil {
			t.Fatal(err)
		}
		dll, err := io.ReadAll(f)
		f.Close()
		if err != nil {
			t.Fatal(err)
		}
		if err := verifyWintunArchitecture(dll, arch); err != nil {
			t.Fatal(err)
		}
		dll[len(dll)/2] ^= 1
		if err := verifyWintunArchitecture(dll, arch); err == nil {
			t.Fatal("accepted one changed byte", arch)
		}
	}
	b[len(b)/2] ^= 1
	if err := verifyWintunArchive(b); err == nil {
		t.Fatal("accepted modified archive")
	}
}

package main

import (
	"os"
	"reflect"
	"regexp"
	"sort"
	"strings"
	"testing"
)

// The functions whose input or output carries a credential (functions.md §1.2).
var wantNotObservable = []string{
	"whatsapp::cloud::phone_numbers::register",
	"whatsapp::cloud::phone_numbers::set_pin",
	"whatsapp::cloud::phone_numbers::verify_code",
	"whatsapp::cloud::webhook::verify",
	"whatsapp::on-config-change",
	"whatsapp::sessions::pair_code",
	"whatsapp::sessions::qr",
}

func TestCredentialFunctionsAreNotObservable(t *testing.T) {
	registered, _ := bootFrames(t, 0)
	var notObservable []string
	for id, frame := range registered {
		switch string(frame["observable"]) {
		case "":
		case "false":
			notObservable = append(notObservable, id)
		default:
			t.Errorf("%s registers observable: %s", id, frame["observable"])
		}
	}
	sort.Strings(notObservable)
	if !reflect.DeepEqual(notObservable, wantNotObservable) {
		t.Fatalf("functions registered with observable: false = %v, want %v", notObservable, wantNotObservable)
	}
	t.Run("README lists them", func(t *testing.T) {
		readme, err := os.ReadFile("README.md")
		if err != nil {
			t.Fatal(err)
		}
		_, section, ok := strings.Cut(string(readme), "### Functions nobody can observe\n")
		if !ok {
			t.Fatal("README has no section for the functions nobody can observe")
		}
		section, _, _ = strings.Cut(section, "\n## ")
		var listed []string
		for _, match := range regexp.MustCompile("(?m)^\\| `(whatsapp::[^`]+)` \\|").FindAllStringSubmatch(section, -1) {
			listed = append(listed, match[1])
		}
		sort.Strings(listed)
		if !reflect.DeepEqual(listed, wantNotObservable) {
			t.Fatalf("README lists %v as not observable, want %v", listed, wantNotObservable)
		}
	})
}

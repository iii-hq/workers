package main

import (
	"os"
	"reflect"
	"regexp"
	"sort"
	"strings"
	"testing"
)

// emittedNativeKinds reads the kinds the worker passes to Manager.emit.
func emittedNativeKinds(t *testing.T) []string {
	t.Helper()
	emit := regexp.MustCompile(`m\.emit\(s, "([a-z_.]+)"`)
	seen := map[string]bool{}
	for _, file := range []string{"events.go", "sessions.go"} {
		source, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		for _, match := range emit.FindAllStringSubmatch(string(source), -1) {
			seen[match[1]] = true
		}
	}
	kinds := make([]string, 0, len(seen))
	for kind := range seen {
		kinds = append(kinds, kind)
	}
	sort.Strings(kinds)
	if len(kinds) == 0 {
		t.Fatal("found no native emit sites")
	}
	return kinds
}

func TestNativeEventKindsAreDocumented(t *testing.T) {
	emitted := emittedNativeKinds(t)
	readme, err := os.ReadFile("README.md")
	if err != nil {
		t.Fatal(err)
	}
	_, sentence, ok := strings.Cut(string(readme), "Native kinds are ")
	if !ok {
		t.Fatal("README does not list the native kinds")
	}
	sentence, _, _ = strings.Cut(sentence, ". ")
	var documented []string
	for _, match := range regexp.MustCompile("`([a-z_.]+)`").FindAllStringSubmatch(sentence, -1) {
		documented = append(documented, match[1])
	}
	sort.Strings(documented)
	if !reflect.DeepEqual(documented, emitted) {
		t.Fatalf("README native kinds = %v, the worker emits %v", documented, emitted)
	}
	t.Run("the ADE session list observes emitted kinds", func(t *testing.T) {
		page, err := os.ReadFile("ui/src/page.tsx")
		if err != nil {
			t.Fatal(err)
		}
		match := regexp.MustCompile(`const SESSION_LIST_KINDS = \[([^\]]*)\]`).FindSubmatch(page)
		if match == nil {
			t.Fatal("ui/src/page.tsx has no SESSION_LIST_KINDS")
		}
		known := map[string]bool{}
		for _, kind := range emitted {
			known[kind] = true
		}
		for _, kind := range regexp.MustCompile(`'([^']+)'`).FindAllSubmatch(match[1], -1) {
			if !known[string(kind[1])] {
				t.Errorf("the session list observes %s, which the worker never emits", kind[1])
			}
		}
	})
}

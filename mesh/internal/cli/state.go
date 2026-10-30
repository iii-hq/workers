package cli

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"regexp"

	"github.com/iii-hq/workers/mesh/internal/join"
)

type State struct {
	PublicURL     string        `json:"public_url"`
	Secret        string        `json:"secret"`
	Identity      join.Identity `json:"hub_identity"`
	Operator      string        `json:"operator"`
	RequestedName string        `json:"requested_name"`
	Name          string        `json:"name,omitempty"`
	Port          int           `json:"port"`
}

var validName = regexp.MustCompile(`^[a-z][a-z0-9-]{0,30}$`)

func (s State) validate() error {
	if err := validateURL(s.PublicURL); err != nil {
		return err
	}
	if err := s.Identity.Verify(s.Identity.Identity); err != nil {
		return err
	}
	if s.Secret == "" || s.Operator == "" || s.Operator == "root" || !validName.MatchString(s.RequestedName) || s.Port < 0 || s.Port > 65535 || s.Name != "" && !validName.MatchString(s.Name) {
		return errors.New("invalid saved node state")
	}
	return nil
}
func privateDir(dir string) error {
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	st, err := os.Lstat(dir)
	if err != nil {
		return err
	}
	if !st.IsDir() || st.Mode()&os.ModeSymlink != 0 {
		return errors.New("node state must be a private directory")
	}
	return os.Chmod(dir, 0700)
}
func writePrivate(dir, name string, b []byte) error {
	if err := privateDir(dir); err != nil {
		return err
	}
	f, err := os.CreateTemp(dir, ".state-*")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	defer f.Close()
	if _, err = f.Write(b); err != nil {
		return err
	}
	if err = f.Sync(); err != nil {
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	return os.Rename(f.Name(), filepath.Join(dir, name))
}
func saveState(dir string, s State) error {
	if err := s.validate(); err != nil {
		return err
	}
	b, err := json.Marshal(s)
	if err != nil {
		return err
	}
	return writePrivate(dir, "join.json", b)
}
func loadState(dir string) (State, error) {
	var s State
	b, err := os.ReadFile(filepath.Join(dir, "join.json"))
	if err != nil {
		return s, err
	}
	if err = json.Unmarshal(b, &s); err != nil {
		return s, err
	}
	return s, s.validate()
}
func exists(path string) bool { _, err := os.Stat(path); return !os.IsNotExist(err) }

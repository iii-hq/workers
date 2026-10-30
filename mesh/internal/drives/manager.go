// MIT License
//
// Copyright (c) 2026 Alexander Yue
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

package drives

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"maps"
	"net/netip"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"
)

// Host is the privileged OS boundary. Run must honor cancellation and execute
// Operator commands in the saved operator's authenticated session.
type Host interface {
	Run(context.Context, Command) (string, error)
	ReadFile(string) ([]byte, error)
	WriteFile(string, []byte, fs.FileMode) error
	MkdirAll(string, fs.FileMode) error
	Remove(string) error
	RemoveAll(string) error
	Glob(string) ([]string, error)
	Chown(string, int, int) error
	LookPath(string) (string, error)
	Mounts(context.Context) (map[string]string, error)
	Reach(context.Context, netip.Addr, string) bool
}
type Command struct {
	Program   string
	Args      []string
	Input     string
	Operator  bool
	Sensitive bool
}
type Options struct {
	StateDir, Platform, Operator string
	UID, GID                     int
}
type ownedMount struct {
	Name   string `json:"name"`
	Target string `json:"target"`
	Source string `json:"source"`
	Host   string `json:"host,omitempty"`
}
type journal struct {
	Mounts map[string]ownedMount `json:"mounts"`
	Server bool                  `json:"server"`
	Users  []string              `json:"users,omitempty"`
	Masks  []string              `json:"masks,omitempty"`
}
type Manager struct {
	mu            sync.Mutex
	opts          Options
	host          Host
	owned         journal
	want          *Desired
	results       []Result
	ctx           context.Context
	cancel        context.CancelFunc
	done          chan struct{}
	closed        bool
	serverApplied string
}

func NewWithHost(o Options, h Host) (*Manager, error) {
	if h == nil || o.StateDir == "" || o.Operator == "" || (o.Platform != "linux" && o.Platform != "darwin" && o.Platform != "windows") {
		return nil, errors.New("drives require state, platform, operator and host")
	}
	m := &Manager{opts: o, host: h, owned: journal{Mounts: map[string]ownedMount{}}, results: []Result{}, done: make(chan struct{})}
	b, err := os.ReadFile(filepath.Join(o.StateDir, "drives.json"))
	if err == nil {
		if err = json.Unmarshal(b, &m.owned); err != nil {
			return nil, fmt.Errorf("drive ownership journal: %w", err)
		}
	} else if !errors.Is(err, fs.ErrNotExist) {
		return nil, err
	}
	if m.owned.Mounts == nil {
		m.owned.Mounts = map[string]ownedMount{}
	}
	for name, r := range m.owned.Mounts {
		if name != r.Name || !ValidDrive(name) || !m.validOwned(r) {
			return nil, errors.New("invalid drive ownership journal")
		}
	}
	if m.owned.Server && o.Platform != "linux" {
		return nil, errors.New("invalid server ownership")
	}
	for _, unit := range m.owned.Masks {
		if !slices.Contains(sambaUnits, unit) {
			return nil, errors.New("invalid Samba mask ownership")
		}
	}
	if !m.owned.Server && (len(m.owned.Users) > 0 || len(m.owned.Masks) > 0) {
		return nil, errors.New("inconsistent server ownership")
	}
	for _, u := range m.owned.Users {
		if !validSMBUser(u) {
			return nil, errors.New("invalid SMB ownership")
		}
	}
	m.ctx, m.cancel = context.WithCancel(context.Background())
	go m.loop()
	return m, nil
}
func (m *Manager) loop() {
	defer close(m.done)
	ticker := time.NewTicker(10 * time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-m.ctx.Done():
			return
		case <-ticker.C:
			m.mu.Lock()
			if !m.closed && m.want != nil {
				m.results = m.reconcile(m.ctx, *m.want, false)
			}
			m.mu.Unlock()
		}
	}
}
func (m *Manager) Close() error {
	m.cancel()
	<-m.done
	m.mu.Lock()
	m.closed = true
	m.mu.Unlock()
	return nil
}
func (m *Manager) Status() []Result { m.mu.Lock(); defer m.mu.Unlock(); return slices.Clone(m.results) }
func (m *Manager) Apply(ctx context.Context, d Desired) []Result {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	stop := context.AfterFunc(m.ctx, cancel)
	defer stop()
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.closed || m.ctx.Err() != nil {
		return []Result{{State: "error", Error: "drive manager is closed"}}
	}
	// Keep credentials only in memory; the journal records resources, not intent.
	b, _ := json.Marshal(d)
	var copy Desired
	_ = json.Unmarshal(b, &copy)
	if err := validate(copy); err != nil {
		m.results = []Result{{State: "error", Error: err.Error()}}
		return slices.Clone(m.results)
	}
	m.want = &copy
	m.results = m.reconcile(ctx, copy, false)
	return slices.Clone(m.results)
}
func (m *Manager) Drop(ctx context.Context) error {
	m.cancel()
	m.mu.Lock()
	defer m.mu.Unlock()
	m.closed = true
	m.want = nil
	m.results = m.reconcile(ctx, Desired{}, true)
	var errs []error
	for _, r := range m.results {
		if r.Error != "" {
			errs = append(errs, fmt.Errorf("drive %s: %s", r.Name, r.Error))
		}
	}
	return errors.Join(errs...)
}
func (m *Manager) save() error {
	if err := os.MkdirAll(m.opts.StateDir, 0700); err != nil {
		return err
	}
	b, err := json.Marshal(m.owned)
	if err != nil {
		return err
	}
	f, err := os.CreateTemp(m.opts.StateDir, ".drives-*")
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
	return os.Rename(f.Name(), filepath.Join(m.opts.StateDir, "drives.json"))
}
func validate(d Desired) error {
	if len(d.Serve)+len(d.Mount) == 0 {
		return nil
	}
	if !d.Prefix.IsValid() || !d.Prefix.Addr().Is6() || d.Prefix.Bits() != 48 || d.Prefix.Masked() != d.Prefix || d.Prefix.Addr().As16()[0] != 0xfd {
		return errors.New("drives require the mesh ULA /48")
	}
	seen := map[string]bool{}
	for _, e := range d.Serve {
		if !ValidDrive(e.Name) || seen[e.Name] {
			return errors.New("invalid or duplicate export")
		}
		seen[e.Name] = true
		for _, c := range e.Clients {
			if !d.Prefix.Contains(c.Addr) || !validCredential(c.SMB) {
				return errors.New("invalid export member address or credential")
			}
		}
	}
	seen = map[string]bool{}
	for _, mt := range d.Mount {
		if !ValidDrive(mt.Name) || seen[mt.Name] || !ValidDrive(mt.Server) || mt.Host != mt.Server+".iii.internal" || !d.Prefix.Contains(mt.Addr) || !validCredential(mt.SMB) {
			return errors.New("invalid drive mount")
		}
		seen[mt.Name] = true
	}
	return nil
}
func validSMBUser(u string) bool {
	return len(u) <= 32 && strings.HasPrefix(u, "iii-") && ValidDrive(u)
}
func validCredential(s *SMB) bool {
	if s == nil {
		return true
	}
	if !validSMBUser(s.User) || len(s.Password) != 64 {
		return false
	}
	for _, c := range s.Password {
		if !(c >= '0' && c <= '9' || c >= 'a' && c <= 'f') {
			return false
		}
	}
	return true
}
func (m *Manager) validOwned(r ownedMount) bool {
	if m.opts.Platform == "windows" {
		return len(r.Target) == 2 && r.Target[0] >= 'D' && r.Target[0] <= 'W' && r.Target[1] == ':' && strings.HasSuffix(r.Host, ".iii.internal") && ValidDrive(strings.TrimSuffix(r.Host, ".iii.internal")) && r.Source == `\\`+r.Host+`\`+r.Name
	}
	if r.Target != mountTarget(m.opts.Platform, r.Name) {
		return false
	}
	if m.opts.Platform == "linux" && r.Source == exportPath(r.Name) {
		return true
	}
	host, path, ok := strings.Cut(r.Source, "]:")
	if ok {
		a, err := netip.ParseAddr(strings.TrimPrefix(host, "["))
		return m.opts.Platform == "linux" && err == nil && a.Is6() && a.As16()[0] == 0xfd && path == exportPath(r.Name)
	}
	return m.opts.Platform == "darwin" && r.Source == r.Host+":"+exportPath(r.Name) && strings.HasSuffix(r.Host, ".iii.internal") && ValidDrive(strings.TrimSuffix(r.Host, ".iii.internal"))
}
func exportPath(name string) string { return "/srv/iii-mesh/" + name }
func mountTarget(platform, name string) string {
	if platform == "darwin" {
		return "/Volumes/" + name
	}
	return "/drives/" + name
}
func (m *Manager) command(ctx context.Context, program string, args ...string) error {
	_, err := m.host.Run(ctx, Command{Program: program, Args: args, Operator: m.opts.Platform == "windows"})
	return err
}
func (m *Manager) reconcile(ctx context.Context, d Desired, force bool) []Result {
	results := m.serve(ctx, d)
	if len(d.Mount) == 0 && len(m.owned.Mounts) == 0 {
		return results
	}
	at, err := m.host.Mounts(ctx)
	if err != nil {
		return append(results, Result{State: "error", Error: err.Error()})
	}
	wanted := map[string]Mount{}
	for _, mt := range d.Mount {
		wanted[mt.Name] = mt
	}
	for _, name := range slices.Sorted(maps.Keys(m.owned.Mounts)) {
		r := m.owned.Mounts[name]
		mt, ok := wanted[name]
		own := slices.ContainsFunc(d.Serve, func(e Export) bool { return e.Name == name })
		source := m.source(mt, own)
		if !ok || r.Source != source {
			if err := m.unmount(ctx, r, at, force); err != nil {
				results = append(results, Result{Name: name, Path: r.Target, State: "error", Error: err.Error()})
			}
		}
	}
	for _, mt := range d.Mount {
		own := slices.ContainsFunc(d.Serve, func(e Export) bool { return e.Name == mt.Name })
		served := slices.ContainsFunc(results, func(r Result) bool { return r.Name == mt.Name && r.Serve && r.State == "serving" })
		results = append(results, m.mount(ctx, mt, own, served, at))
	}
	return results
}
func (m *Manager) source(mt Mount, own bool) string {
	if m.opts.Platform == "windows" {
		return `\\` + mt.Host + `\` + mt.Name
	}
	if own {
		return exportPath(mt.Name)
	}
	if m.opts.Platform == "darwin" {
		return mt.Host + ":" + exportPath(mt.Name)
	}
	return "[" + mt.Addr.String() + "]:" + exportPath(mt.Name)
}
func (m *Manager) mount(ctx context.Context, mt Mount, own, served bool, at map[string]string) Result {
	r := Result{Name: mt.Name, State: "error", Path: mountTarget(m.opts.Platform, mt.Name)}
	if own && !served {
		r.Error = "local export is unavailable"
		return r
	}
	if m.opts.Platform == "windows" && mt.SMB == nil {
		r.Error = "missing SMB credential"
		return r
	}
	source := m.source(mt, own)
	saved, ok := m.owned.Mounts[mt.Name]
	if ok {
		r.Path = saved.Target
		if saved.Source != source {
			r.Error = "previous source could not be unmounted"
			return r
		}
	} else if m.opts.Platform == "windows" {
		r.Path = ""
		for letter := 'W'; letter >= 'D'; letter-- {
			p := string(letter) + ":"
			taken := slices.ContainsFunc(slices.Collect(maps.Values(m.owned.Mounts)), func(o ownedMount) bool { return o.Target == p })
			if !taken && at[p] == "" {
				r.Path = p
				break
			}
		}
		if r.Path == "" {
			r.Error = "no free drive letter from W: to D:"
			return r
		}
	}
	if src := at[r.Path]; src != "" && (!m.sameSource(src, source) || !ok) {
		r.Error = "mount target is occupied by an unowned resource"
		return r
	}
	port := "2049"
	if m.opts.Platform == "windows" {
		port = "445"
	}
	if !own && !m.host.Reach(ctx, mt.Addr, port) {
		r.State = "waiting"
		r.Error = "server unreachable; file operations wait"
		return r
	}
	if m.sameSource(at[r.Path], source) {
		r.State = "mounted"
		return r
	}
	if !ok {
		m.owned.Mounts[mt.Name] = ownedMount{Name: mt.Name, Target: r.Path, Source: source, Host: mt.Host}
		if err := m.save(); err != nil {
			delete(m.owned.Mounts, mt.Name)
			r.Error = err.Error()
			return r
		}
	}
	var err error
	if m.opts.Platform == "windows" {
		_, err = m.host.Run(ctx, Command{Program: "cmdkey", Args: []string{"/add:" + mt.Host, "/user:" + mt.SMB.User, "/pass:" + mt.SMB.Password}, Operator: true, Sensitive: true})
		if err == nil {
			err = m.command(ctx, "net", "use", r.Path, source, "/persistent:no")
		}
	} else {
		err = m.host.MkdirAll(r.Path, 0555)
		if err == nil {
			opts := "hard,actimeo=1"
			kind := "nfs4"
			if m.opts.Platform == "darwin" {
				opts = "vers=4.0,hard,intr,actimeo=1,resvport,inet6"
				kind = "nfs"
			}
			args := []string{"-t", kind, "-o", opts, source, r.Path}
			if own {
				args = []string{"-o", "bind", source, r.Path}
			} else if m.opts.Platform == "linux" {
				if _, e := m.host.LookPath("mount.nfs4"); e != nil {
					err = m.apt(ctx, "nfs-common")
				}
			}
			if err == nil {
				err = m.command(ctx, "mount", args...)
			}
		}
	}
	if err != nil {
		r.Error = err.Error()
		return r
	}
	now, err := m.host.Mounts(ctx)
	if err != nil {
		r.Error = err.Error()
		return r
	}
	if !m.sameSource(now[r.Path], source) {
		r.Error = "mount command did not establish the requested source"
		return r
	}
	at[r.Path] = source
	r.State = "mounted"
	return r
}
func (m *Manager) unmount(ctx context.Context, r ownedMount, at map[string]string, force bool) error {
	if src := at[r.Target]; src != "" && !m.sameSource(src, r.Source) {
		return errors.New("owned target was replaced by a foreign mount")
	}
	if at[r.Target] != "" {
		args := []string{r.Target}
		program := "umount"
		switch m.opts.Platform {
		case "linux":
			args = []string{"-i", "-c", r.Target}
			if force {
				args = []string{"-i", "-c", "-f", "-l", r.Target}
			}
		case "darwin":
			if force {
				args = []string{"-f", r.Target}
			}
		case "windows":
			program = "net"
			args = []string{"use", r.Target, "/delete"}
			if force {
				args = append(args, "/y")
			}
		}
		err := m.command(ctx, program, args...)
		now, checkErr := m.host.Mounts(ctx)
		if checkErr != nil {
			return checkErr
		}
		if now[r.Target] != "" {
			if err != nil {
				return err
			}
			return errors.New("resource remains mounted")
		}
		delete(at, r.Target)
	}
	if m.opts.Platform == "windows" {
		another := false
		for name, o := range m.owned.Mounts {
			another = another || name != r.Name && o.Host == r.Host
		}
		if !another {
			out, err := m.host.Run(ctx, Command{Program: "cmdkey", Args: []string{"/list"}, Operator: true})
			if err != nil {
				return err
			}
			if strings.Contains(out, r.Host) {
				if err = m.command(ctx, "cmdkey", "/delete:"+r.Host); err != nil {
					return err
				}
			}
		}
	} else if err := m.host.Remove(r.Target); err != nil && !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	delete(m.owned.Mounts, r.Name)
	if err := m.save(); err != nil {
		m.owned.Mounts[r.Name] = r
		return err
	}
	return nil
}

func (m *Manager) sameSource(a, b string) bool {
	if m.opts.Platform == "windows" {
		return strings.EqualFold(a, b)
	}
	return a == b
}

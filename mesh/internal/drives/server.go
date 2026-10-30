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
	"path/filepath"
	"slices"
	"strconv"
	"strings"
)

const exportsFile = "/etc/exports.d/iii-mesh.exports"
const nfsConf = "/etc/nfs.conf.d/iii-mesh.conf"
const smbUnit = "/etc/systemd/system/iii-mesh-smbd.service"
const smbService = "iii-mesh-smbd"

var sambaUnits = []string{"smbd.service", "nmbd.service", "samba-ad-dc.service"}

const smbComment = "iii mesh drive client"
const nfsConfig = `[nfsd]
vers3=n
vers4=y
vers4.0=y
vers4.1=y
vers4.2=y
lease-time=20
grace-time=20
`
const smbConfig = `[global]
 server role = standalone server
 smb ports = 445
 disable netbios = yes
 server min protocol = SMB3
 map to guest = never
 restrict anonymous = 2
 load printers = no
 printcap name = /dev/null
 disable spoolss = yes
 passdb backend = tdbsam:%[1]s/passdb.tdb
 private dir = %[1]s/private
 lock directory = %[1]s/lock
 state directory = %[1]s/state
 cache directory = %[1]s/cache
 pid directory = %[1]s
 ncalrpc dir = %[1]s/ncalrpc
 log file = %[1]s/log
 max log size = 1000
 posix locking = yes
 kernel oplocks = yes
 force user = %[2]s
`

func (m *Manager) smbDir() string  { return filepath.Join(m.opts.StateDir, "smb") }
func (m *Manager) smbConf() string { return filepath.Join(m.smbDir(), "smb.conf") }
func (m *Manager) serve(ctx context.Context, d Desired) []Result {
	if len(d.Serve) == 0 {
		if err := m.unserve(ctx); err != nil {
			return []Result{{Serve: true, State: "error", Error: err.Error()}}
		}
		return nil
	}
	results := make([]Result, 0, len(d.Serve))
	var err error
	if m.opts.Platform != "linux" {
		err = errors.New("only a Linux node with apt can serve drives")
	} else {
		err = m.setupServer(ctx, d)
	}
	for _, e := range d.Serve {
		r := Result{Name: e.Name, Path: exportPath(e.Name), Serve: true, State: "serving"}
		if err != nil {
			r.State = "error"
			r.Error = err.Error()
		}
		results = append(results, r)
	}
	return results
}
func (m *Manager) preflight(ctx context.Context) error {
	if _, err := m.host.LookPath("apt-get"); err != nil {
		return errors.New("refused: a drive server requires Linux with apt")
	}
	files, err := m.host.Glob("/etc/exports.d/*.exports")
	if err != nil {
		return err
	}
	for _, p := range append([]string{"/etc/exports"}, files...) {
		b, err := m.host.ReadFile(p)
		if errors.Is(err, fs.ErrNotExist) {
			continue
		}
		if err != nil {
			return err
		}
		for line := range strings.Lines(string(b)) {
			line = strings.TrimSpace(line)
			if line != "" && !strings.HasPrefix(line, "#") {
				return fmt.Errorf("refused: existing NFS exports in %s", p)
			}
		}
	}
	if _, err := m.host.LookPath("exportfs"); err == nil {
		out, err := m.host.Run(ctx, Command{Program: "exportfs"})
		if err != nil {
			return err
		}
		if strings.TrimSpace(out) != "" {
			return errors.New("refused: existing active NFS exports")
		}
	}
	out, err := m.host.Run(ctx, Command{Program: "ss", Args: []string{"-Hltnp", "sport", "=", ":445"}})
	if err != nil {
		return err
	}
	if strings.TrimSpace(out) != "" {
		return errors.New("refused: another program listens on port 445")
	}
	return nil
}
func (m *Manager) apt(ctx context.Context, pkg string) error {
	if _, err := m.host.LookPath("apt-get"); err != nil {
		return errors.New("apt is required to install drive dependencies")
	}
	if err := m.command(ctx, "apt-get", "update"); err != nil {
		return err
	}
	return m.command(ctx, "apt-get", "-o", "DPkg::Options::=--force-confold", "install", "-y", "--no-install-recommends", pkg)
}
func (m *Manager) write(path, text string, mode fs.FileMode) (bool, error) {
	if b, err := m.host.ReadFile(path); err == nil && string(b) == text {
		return false, nil
	}
	if err := m.host.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return false, err
	}
	return true, m.host.WriteFile(path, []byte(text), mode)
}
func (m *Manager) setupServer(ctx context.Context, d Desired) error {
	signature, _ := json.Marshal(struct {
		Prefix  string
		Exports []Export
	}{d.Prefix.String(), d.Serve})
	if m.serverApplied == string(signature) {
		return m.command(ctx, "systemctl", "start", smbService)
	}
	if m.owned.Server {
		if _, err := m.host.ReadFile(smbUnit); err == nil {
			if err = m.command(ctx, "systemctl", "stop", smbService); err != nil {
				return err
			}
		} else if !errors.Is(err, fs.ErrNotExist) {
			return err
		}
	}
	if m.opts.UID <= 0 || m.opts.GID < 0 || strings.ContainsAny(m.opts.Operator, "\r\n\t[]=%") {
		return errors.New("invalid server operator")
	}
	if !m.owned.Server {
		if err := m.preflight(ctx); err != nil {
			return err
		}
		m.owned.Server = true
		if err := m.save(); err != nil {
			m.owned.Server = false
			return err
		}
	}
	if _, err := m.host.LookPath("exportfs"); err != nil {
		if err = m.apt(ctx, "nfs-kernel-server"); err != nil {
			return err
		}
	}
	if _, err := m.host.LookPath("smbd"); err != nil {
		if err = m.maskSamba(ctx); err != nil {
			return err
		}
		installErr := m.apt(ctx, "samba")
		stopErr := m.command(ctx, "systemctl", "disable", "--now", "smbd.service", "nmbd.service", "samba-ad-dc.service")
		if err = errors.Join(installErr, stopErr); err != nil {
			return err
		}
		if err = m.unmaskSamba(ctx); err != nil {
			return err
		}
	}
	if err := m.unmaskSamba(ctx); err != nil {
		return err
	}
	changed, err := m.write(nfsConf, nfsConfig, 0644)
	if err != nil {
		return err
	}
	if changed {
		if err = m.command(ctx, "systemctl", "restart", "nfs-server"); err != nil {
			_ = m.host.Remove(nfsConf)
			return err
		}
	}
	lines := []string{"# iii mesh owned exports"}
	shares := ""
	users := map[string]string{}
	for _, e := range d.Serve {
		path := exportPath(e.Name)
		if err = m.host.MkdirAll(path, 0755); err != nil {
			return err
		}
		if err = m.host.Chown(path, m.opts.UID, m.opts.GID); err != nil {
			return err
		}
		if len(e.Clients) == 0 {
			continue
		}
		line := path
		var valid, hosts []string
		for _, c := range e.Clients {
			line += fmt.Sprintf(" %s(rw,sync,all_squash,anonuid=%d,anongid=%d,no_subtree_check)", c.Addr, m.opts.UID, m.opts.GID)
			if c.SMB != nil {
				valid = append(valid, c.SMB.User)
				hosts = append(hosts, c.Addr.String())
				users[c.SMB.User] = c.SMB.Password
			}
		}
		lines = append(lines, line)
		if len(valid) > 0 {
			shares += fmt.Sprintf("[%s]\n path = %s\n read only = no\n valid users = %s\n hosts allow = %s\n", e.Name, path, strings.Join(valid, " "), strings.Join(hosts, " "))
		}
	}
	_, err = m.write(exportsFile, strings.Join(lines, "\n")+"\n", 0644)
	if err != nil {
		return err
	}
	// Reapply even after a prior exportfs failure; writing the file is not success.
	if err = m.command(ctx, "exportfs", "-ra"); err != nil {
		return err
	}
	for _, sub := range []string{"", "private", "lock", "state", "cache", "ncalrpc"} {
		if err = m.host.MkdirAll(filepath.Join(m.smbDir(), sub), 0700); err != nil {
			return err
		}
	}
	unit := fmt.Sprintf("[Unit]\nDescription=iii mesh shared drives\n[Service]\nExecStart=/usr/sbin/smbd --foreground --no-process-group --configfile=%s\nRestart=always\nRestartSec=2\nIPAddressDeny=any\nIPAddressAllow=%s\n", strconv.Quote(m.smbConf()), d.Prefix)
	unitChanged, err := m.write(smbUnit, unit, 0644)
	if err != nil {
		return err
	}
	if unitChanged {
		if err = m.command(ctx, "systemctl", "daemon-reload"); err != nil {
			return err
		}
	}
	_, err = m.write(m.smbConf(), fmt.Sprintf(smbConfig, m.smbDir(), m.opts.Operator)+shares, 0600)
	if err != nil {
		return err
	}
	// A restart revokes already-open shares after membership changes.
	if err = m.smbUsers(ctx, users); err != nil {
		return err
	}
	if err = m.command(ctx, "systemctl", "restart", smbService); err != nil {
		return err
	}
	m.serverApplied = string(signature)
	return nil
}
func (m *Manager) smbUsers(ctx context.Context, users map[string]string) error {
	passwd, err := m.host.ReadFile("/etc/passwd")
	if err != nil && !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	accounts := map[string]string{}
	for line := range strings.Lines(string(passwd)) {
		f := strings.Split(line, ":")
		if len(f) > 4 {
			accounts[f[0]] = f[4]
		}
	}
	for _, u := range slices.Clone(m.owned.Users) {
		if _, ok := users[u]; ok {
			continue
		}
		if comment, ok := accounts[u]; ok && comment != smbComment {
			return fmt.Errorf("refused: owned SMB account %s was replaced", u)
		}
		if _, err := m.host.ReadFile(m.smbConf()); err == nil {
			out, err := m.host.Run(ctx, Command{Program: "pdbedit", Args: []string{"-s", m.smbConf(), "-L"}})
			if err != nil {
				return err
			}
			for line := range strings.Lines(out) {
				if strings.HasPrefix(line, u+":") {
					if err = m.command(ctx, "smbpasswd", "-c", m.smbConf(), "-x", u); err != nil {
						return err
					}
					break
				}
			}
		} else if !errors.Is(err, fs.ErrNotExist) {
			return err
		}
		if _, ok := accounts[u]; ok {
			if err := m.command(ctx, "userdel", u); err != nil {
				return err
			}
		}
		old := slices.Clone(m.owned.Users)
		m.owned.Users = slices.DeleteFunc(m.owned.Users, func(s string) bool { return s == u })
		if err := m.save(); err != nil {
			m.owned.Users = old
			return err
		}
	}
	for _, u := range slices.Sorted(maps.Keys(users)) {
		comment, found := accounts[u]
		owned := slices.Contains(m.owned.Users, u)
		if found && (comment != smbComment || !owned) {
			return fmt.Errorf("refused: SMB account %s belongs to another service", u)
		}
		if !owned {
			old := slices.Clone(m.owned.Users)
			m.owned.Users = append(m.owned.Users, u)
			if err = m.save(); err != nil {
				m.owned.Users = old
				return err
			}
		}
		if !found {
			if err = m.command(ctx, "useradd", "--system", "--no-create-home", "--home-dir", "/nonexistent", "--shell", "/usr/sbin/nologin", "--comment", smbComment, u); err != nil {
				return err
			}
		}
		pw := users[u]
		if _, err = m.host.Run(ctx, Command{Program: "smbpasswd", Args: []string{"-c", m.smbConf(), "-s", "-a", u}, Input: pw + "\n" + pw + "\n", Sensitive: true}); err != nil {
			return err
		}
	}
	return nil
}
func (m *Manager) maskSamba(ctx context.Context) error {
	for _, unit := range sambaUnits {
		if slices.Contains(m.owned.Masks, unit) {
			continue
		}
		if _, err := m.host.ReadFile("/run/systemd/system/" + unit); err == nil {
			continue
		} else if !errors.Is(err, fs.ErrNotExist) {
			return err
		}
		old := slices.Clone(m.owned.Masks)
		m.owned.Masks = append(m.owned.Masks, unit)
		if err := m.save(); err != nil {
			m.owned.Masks = old
			return err
		}
		if err := m.command(ctx, "systemctl", "mask", "--runtime", unit); err != nil {
			return err
		}
	}
	return nil
}
func (m *Manager) unmaskSamba(ctx context.Context) error {
	for _, unit := range slices.Clone(m.owned.Masks) {
		if err := m.command(ctx, "systemctl", "unmask", "--runtime", unit); err != nil {
			return err
		}
		old := slices.Clone(m.owned.Masks)
		m.owned.Masks = slices.DeleteFunc(m.owned.Masks, func(s string) bool { return s == unit })
		if err := m.save(); err != nil {
			m.owned.Masks = old
			return err
		}
	}
	return nil
}
func (m *Manager) unserve(ctx context.Context) error {
	if !m.owned.Server {
		return nil
	}
	// Retain ownership until every effect is undone, so a crash retries safely.
	if _, err := m.host.ReadFile(smbUnit); err == nil {
		if err = m.command(ctx, "systemctl", "stop", smbService); err != nil {
			return err
		}
	} else if !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	if _, err := m.host.ReadFile(exportsFile); err == nil {
		if err = m.host.WriteFile(exportsFile, nil, 0644); err != nil {
			return err
		}
		if err = m.command(ctx, "exportfs", "-ra"); err != nil {
			return err
		}
	} else if !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	if err := m.smbUsers(ctx, nil); err != nil {
		return err
	}
	if err := m.unmaskSamba(ctx); err != nil {
		return err
	}
	for _, p := range []string{exportsFile, nfsConf, smbUnit, m.smbConf(), filepath.Join(m.smbDir(), "passdb.tdb")} {
		if err := m.host.Remove(p); err != nil && !errors.Is(err, fs.ErrNotExist) {
			return err
		}
	}
	if err := m.host.RemoveAll(m.smbDir()); err != nil {
		return err
	}
	if err := m.command(ctx, "systemctl", "daemon-reload"); err != nil {
		return err
	}
	m.serverApplied = ""
	m.owned.Server = false
	if err := m.save(); err != nil {
		m.owned.Server = true
		return err
	}
	return nil
}

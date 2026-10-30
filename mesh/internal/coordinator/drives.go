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

package coordinator

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"maps"
	"slices"

	"github.com/iii-hq/workers/mesh/internal/drives"
)

type Drive struct {
	Server string   `json:"server"`
	Nodes  []string `json:"nodes"`
}

func (c *Coordinator) SetDrives(ds map[string]Drive) []error {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.drives = make(map[string]Drive, len(ds))
	var errs []error
	for _, name := range slices.Sorted(maps.Keys(ds)) {
		d := ds[name]
		d.Nodes = slices.Clone(d.Nodes)
		c.drives[name] = d
		if _, err := c.driveServer(name, d); err != nil {
			errs = append(errs, err)
		}
	}
	c.wake()
	return errs
}
func (c *Coordinator) driveServer(name string, d Drive) (*node, error) {
	if !drives.ValidDrive(name) {
		return nil, fmt.Errorf("invalid drive name %q", name)
	}
	s := c.byName(d.Server)
	if !c.authorized(s) || s.Name == "hub" || s.OS != "linux" {
		return nil, fmt.Errorf("drive %s: server %s is not a known Linux node", name, d.Server)
	}
	return s, nil
}
func (c *Coordinator) driveMember(d Drive, n *node) bool {
	return c.authorized(n) && n.Name != "hub" && (slices.Contains(d.Nodes, n.Name) || !n.Ephemeral && slices.Contains(d.Nodes, "*"))
}
func (c *Coordinator) serves(a, b *node) bool {
	for name, d := range c.drives {
		if s, err := c.driveServer(name, d); err == nil && s == a && c.driveMember(d, b) {
			return true
		}
	}
	return false
}
func (c *Coordinator) Desired(name string) drives.Desired {
	c.mu.Lock()
	defer c.mu.Unlock()
	want := drives.Desired{Prefix: c.st.Prefix, Serve: []drives.Export{}, Mount: []drives.Mount{}}
	n := c.byName(name)
	if !c.authorized(n) || name == "hub" {
		return want
	}
	for _, dn := range slices.Sorted(maps.Keys(c.drives)) {
		d := c.drives[dn]
		s, err := c.driveServer(dn, d)
		if err != nil {
			continue
		}
		if s == n {
			e := drives.Export{Name: dn, Clients: []drives.Client{}}
			for _, client := range c.st.Nodes {
				if client != s && c.driveMember(d, client) {
					e.Clients = append(e.Clients, drives.Client{Name: client.Name, Addr: c.address(client.ID), SMB: c.smb(s, client)})
				}
			}
			slices.SortFunc(e.Clients, func(a, b drives.Client) int {
				if a.Name < b.Name {
					return -1
				}
				if a.Name > b.Name {
					return 1
				}
				return 0
			})
			want.Serve = append(want.Serve, e)
		}
		if c.driveMember(d, n) {
			want.Mount = append(want.Mount, drives.Mount{Name: dn, Server: s.Name, Host: s.Name + ".iii.internal", Addr: c.address(s.ID), SMB: c.smb(s, n)})
		}
	}
	return want
}
func (c *Coordinator) smb(server, client *node) *drives.SMB {
	if client.OS != "windows" {
		return nil
	}
	h := hmac.New(sha256.New, c.st.Identity.Seed())
	h.Write([]byte("iii-mesh/smb/v1\x00" + server.Name + "\x00" + client.Name))
	user := sha256.Sum256([]byte(client.Name))
	return &drives.SMB{User: "iii-" + hex.EncodeToString(user[:12]), Password: hex.EncodeToString(h.Sum(nil))}
}

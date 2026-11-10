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

package meshnet

import (
	"fmt"
	"net/netip"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"slices"
	"strings"
	"time"

	"tailscale.com/tailcfg"
	"tailscale.com/types/netmap"
)

const (
	begin  = "# iii mesh begin"
	end    = "# iii mesh end"
	Domain = "iii.internal"
)

var blockLines = regexp.MustCompile("(?ms)^" + begin + "\r?\n.*?^" + end + "\r?(?:\n|\\z)")
var nodeName = regexp.MustCompile(`^[a-z][a-z0-9-]{0,30}$`)

func hostsPath() string {
	return platformHostsPath()
}

func names(nm *netmap.NetworkMap) map[string]netip.Addr {
	result := make(map[string]netip.Addr)
	if nm != nil {
		for _, n := range append([]tailcfg.NodeView{nm.SelfNode}, nm.Peers...) {
			if n.Valid() && n.Addresses().Len() > 0 {
				result[n.Name()] = n.Addresses().At(0).Addr()
			}
		}
	}
	return result
}

// An empty path is a userspace node, which owns no hosts file.
func WriteHostsBlock(path string, names map[string]netip.Addr) error {
	if path == "" {
		return nil
	}
	keys := make([]string, 0, len(names))
	for name, addr := range names {
		if !nodeName.MatchString(name) || !addr.IsValid() || addr.Zone() != "" {
			return fmt.Errorf("invalid hosts entry %q", name)
		}
		keys = append(keys, name)
	}
	slices.Sort(keys)
	old, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	stat, err := os.Stat(path)
	if err != nil {
		return err
	}
	nl := "\n"
	if strings.Contains(string(old), "\r\n") {
		nl = "\r\n"
	}
	var b strings.Builder
	if len(keys) > 0 {
		b.WriteString(begin + nl)
		for _, name := range keys {
			fmt.Fprintf(&b, "%s %s.%s%s", names[name], name, Domain, nl)
		}
		b.WriteString(end + nl)
	}
	content := string(old)
	// Count every marker before matching: a valid block must not hide a
	// nested or trailing marker elsewhere in the file.
	if strings.Count(content, begin) != strings.Count(content, end) || strings.Count(content, begin) > 1 {
		return fmt.Errorf("malformed mesh hosts markers")
	}
	locations := blockLines.FindAllStringIndex(content, -1)
	if len(locations) > 1 {
		return fmt.Errorf("multiple mesh hosts blocks")
	}
	if len(locations) == 1 {
		loc := locations[0]
		content = content[:loc[0]] + b.String() + content[loc[1]:]
	} else {
		if strings.Contains(content, begin) || strings.Contains(content, end) {
			return fmt.Errorf("incomplete mesh hosts block")
		}
		if b.Len() > 0 {
			if content != "" && !strings.HasSuffix(content, "\n") {
				content += nl
			}
			content += b.String()
		}
	}
	if content == string(old) {
		return nil
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".iii-hosts-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	if err = tmp.Chmod(stat.Mode().Perm()); err == nil {
		_, err = tmp.WriteString(content)
	}
	if err == nil {
		err = tmp.Sync()
	}
	errClose := tmp.Close()
	if err != nil {
		return err
	}
	if errClose != nil {
		return errClose
	}
	attempts := 1
	if runtime.GOOS == "windows" {
		attempts = 20
	}
	for i := 0; i < attempts; i++ {
		err = os.Rename(tmp.Name(), path)
		if err == nil {
			return nil
		}
		if i+1 < attempts {
			time.Sleep(50 * time.Millisecond)
		}
	}
	return err
}

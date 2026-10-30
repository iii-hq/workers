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
	"bytes"
	"context"
	"fmt"
	"io/fs"
	"net"
	"net/netip"
	"os"
	"os/exec"
	"os/user"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"

	"github.com/iii-hq/workers/mesh/internal/supervisor"
)

func New(stateDir, account string) (*Manager, error) {
	op, err := supervisor.LookupOperator(account)
	if err != nil {
		return nil, err
	}
	o := Options{StateDir: stateDir, Platform: runtime.GOOS, Operator: account}
	if runtime.GOOS != "windows" {
		u, err := user.Lookup(account)
		if err != nil {
			return nil, err
		}
		o.Operator = u.Username
		o.UID, err = strconv.Atoi(u.Uid)
		if err != nil {
			return nil, err
		}
		o.GID, err = strconv.Atoi(u.Gid)
		if err != nil {
			return nil, err
		}
	}
	return NewWithHost(o, &nativeHost{operator: op})
}

type nativeHost struct {
	operator supervisor.Operator
	bound    *supervisor.Operator
}

func (h *nativeHost) CaptureLogon() (supervisor.OperatorLogon, error) {
	return supervisor.CaptureOperatorLogon(h.operator)
}
func (h *nativeHost) BindLogon(l supervisor.OperatorLogon) error {
	op, err := supervisor.BindOperatorLogon(h.operator, l)
	if err == nil {
		h.bound = &op
	}
	return err
}

func (h *nativeHost) Run(ctx context.Context, c Command) (string, error) {
	limit := time.Minute
	if c.Program == "apt-get" {
		limit = 10 * time.Minute
	}
	ctx, cancel := context.WithTimeout(ctx, limit)
	defer cancel()
	path, err := commandPath(c.Program)
	if err != nil {
		return "", err
	}
	cmd := exec.CommandContext(ctx, path, c.Args...)
	cmd.WaitDelay = 2 * time.Second
	var b bytes.Buffer
	cmd.Stdout = &b
	cmd.Stderr = &b
	cmd.Stdin = strings.NewReader(c.Input)
	if c.Operator {
		op := h.operator
		if runtime.GOOS == "windows" {
			if h.bound == nil {
				return "", fmt.Errorf("Windows drive command requires an original logon binding")
			}
			op = *h.bound
		}
		err = supervisor.RunOperator(ctx, cmd, op)
	} else {
		cmd.Env = append(os.Environ(), "DEBIAN_FRONTEND=noninteractive")
		err = runPrivileged(cmd)
	}
	if err != nil {
		if c.Sensitive {
			return "", fmt.Errorf("%s failed: %w", c.Program, err)
		}
		return b.String(), fmt.Errorf("%s: %w: %s", c.Program, err, strings.TrimSpace(b.String()))
	}
	if c.Sensitive {
		return "", nil
	}
	return b.String(), nil
}
func (*nativeHost) ReadFile(p string) ([]byte, error) { return os.ReadFile(p) }
func (*nativeHost) WriteFile(p string, b []byte, mode fs.FileMode) error {
	return os.WriteFile(p, b, mode)
}
func (*nativeHost) MkdirAll(p string, mode fs.FileMode) error { return os.MkdirAll(p, mode) }
func (*nativeHost) Remove(p string) error                     { return os.Remove(p) }
func (*nativeHost) RemoveAll(p string) error                  { return os.RemoveAll(p) }
func (*nativeHost) Glob(p string) ([]string, error)           { return filepath.Glob(p) }
func (*nativeHost) Chown(p string, u, g int) error            { return os.Chown(p, u, g) }
func (*nativeHost) LookPath(p string) (string, error)         { return commandPath(p) }
func (*nativeHost) Reach(ctx context.Context, a netip.Addr, port string) bool {
	d := net.Dialer{Timeout: 5 * time.Second}
	c, err := d.DialContext(ctx, "tcp6", net.JoinHostPort(a.String(), port))
	if err != nil {
		return false
	}
	c.Close()
	return true
}

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

package service

import (
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/iii-hq/workers/mesh/internal/elevate"
	"github.com/iii-hq/workers/mesh/internal/supervisor"
)

type Paths struct{ Bin, State, Cache, Unit string }

func DefaultPaths() Paths {
	p := Paths{Bin: "/usr/local/bin/mesh", Cache: supervisor.DefaultCacheDir()}
	switch runtime.GOOS {
	case "linux":
		p.State = "/var/lib/iii-mesh"
		p.Unit = "/etc/systemd/system/iii-mesh.service"
	case "darwin":
		p.State = "/Library/Application Support/iii-mesh"
		p.Unit = "/Library/LaunchDaemons/dev.iii.mesh.plist"
	}
	return platformPaths(p)
}

type manager struct {
	platform string
	paths    Paths
	run      func(string, ...string) error
}

func native() manager { return manager{runtime.GOOS, DefaultPaths(), run} }
func Install(binPath, operator string) error {
	if !elevate.Is() {
		return errors.New("service installation requires root")
	}
	if _, err := supervisor.LookupOperator(operator); err != nil {
		return err
	}
	if runtime.GOOS == "windows" {
		return nativeInstall(binPath)
	}
	return native().install(binPath)
}
func Uninstall() error {
	if !elevate.Is() {
		return errors.New("service removal requires root")
	}
	if runtime.GOOS == "windows" {
		return nativeUninstall()
	}
	return native().uninstall()
}
func run(name string, args ...string) error {
	out, err := exec.Command(name, args...).CombinedOutput()
	if err != nil {
		return fmt.Errorf("%s %s: %w: %s", name, strings.Join(args, " "), err, out)
	}
	return nil
}
func (m manager) install(source string) error {
	if m.platform != "linux" && m.platform != "darwin" {
		return errors.New("service platform unsupported")
	}
	if err := place(source, m.paths.Bin); err != nil {
		return err
	}
	if err := os.MkdirAll(m.paths.State, 0700); err != nil {
		return err
	}
	if err := os.Chmod(m.paths.State, 0700); err != nil {
		return err
	}
	text := SystemdUnit(m.paths.Bin)
	if m.platform == "darwin" {
		if err := m.bootout(); err != nil {
			return err
		}
		text = LaunchdPlist(m.paths.Bin, filepath.Join(m.paths.State, "node.log"))
	}
	if err := os.WriteFile(m.paths.Unit, []byte(text), 0644); err != nil {
		return err
	}
	if m.platform == "darwin" {
		return m.run("launchctl", "bootstrap", "system", m.paths.Unit)
	}
	if err := m.run("systemctl", "daemon-reload"); err != nil {
		return err
	}
	if err := m.run("systemctl", "enable", "iii-mesh"); err != nil {
		return err
	}
	return m.run("systemctl", "restart", "iii-mesh")
}
func (m manager) uninstall() error {
	if m.platform != "linux" && m.platform != "darwin" {
		return errors.New("service platform unsupported")
	}
	if _, err := os.Stat(m.paths.Unit); os.IsNotExist(err) {
		return nil
	} else if err != nil {
		return err
	}
	if m.platform == "darwin" {
		if err := m.bootout(); err != nil {
			return err
		}
	} else if err := m.run("systemctl", "disable", "--now", "iii-mesh"); err != nil {
		return err
	}
	if err := os.Remove(m.paths.Unit); err != nil {
		return err
	}
	if m.platform == "linux" {
		return m.run("systemctl", "daemon-reload")
	}
	return nil
}
func (m manager) bootout() error {
	err := m.run("launchctl", "bootout", "system/dev.iii.mesh")
	deadline := time.Now().Add(30 * time.Second)
	for m.run("launchctl", "print", "system/dev.iii.mesh") == nil {
		if time.Now().After(deadline) {
			return errors.Join(errors.New("mesh daemon is still loaded"), err)
		}
		time.Sleep(100 * time.Millisecond)
	}
	return nil
}
func place(source, destination string) error {
	source, err := filepath.EvalSymlinks(source)
	if err != nil {
		return err
	}
	if source == destination {
		return nil
	}
	in, err := os.Open(source)
	if err != nil {
		return err
	}
	defer in.Close()
	if err := os.MkdirAll(filepath.Dir(destination), 0755); err != nil {
		return err
	}
	out, err := os.CreateTemp(filepath.Dir(destination), ".mesh-*")
	if err != nil {
		return err
	}
	defer os.Remove(out.Name())
	defer out.Close()
	if _, err = io.Copy(out, in); err != nil {
		return err
	}
	if err = out.Chmod(0755); err != nil {
		return err
	}
	if err = out.Sync(); err != nil {
		return err
	}
	if err = out.Close(); err != nil {
		return err
	}
	return os.Rename(out.Name(), destination)
}

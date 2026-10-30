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
	"errors"
	"golang.org/x/sys/windows"
	"os/exec"
	"path/filepath"
)

func commandPath(name string) (string, error) {
	dir, err := windows.GetSystemDirectory()
	if err != nil {
		return "", err
	}
	if name == "powershell" {
		return filepath.Join(dir, "WindowsPowerShell", "v1.0", "powershell.exe"), nil
	}
	return filepath.Join(dir, name+".exe"), nil
}
func (h *nativeHost) Mounts(ctx context.Context) (map[string]string, error) {
	out, err := h.Run(ctx, Command{Program: "net", Args: []string{"use"}, Operator: true})
	if err != nil {
		return nil, err
	}
	at := parseWindowsMappings(out)
	out, err = h.Run(ctx, Command{Program: "powershell", Args: []string{"-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "[System.IO.Directory]::GetLogicalDrives()"}, Operator: true})
	if err != nil {
		return nil, err
	}
	if err = addWindowsDriveLetters(at, out); err != nil {
		return nil, err
	}
	return at, nil
}

func runPrivileged(*exec.Cmd) error {
	return errors.New("Windows drive commands require the authenticated operator session")
}

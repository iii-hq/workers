//go:build windows

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
	"context"
	"errors"
	"log"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/iii-hq/workers/mesh/internal/winsecurity"
	"golang.org/x/sys/windows"
	"golang.org/x/sys/windows/svc"
	"golang.org/x/sys/windows/svc/mgr"
)

const windowsServiceName = "iii-mesh"

func platformPaths(p Paths) Paths {
	data, programs := winsecurity.ProgramData(), winsecurity.ProgramFiles()
	if data == "" || programs == "" {
		return Paths{}
	}
	p.State = filepath.Join(data, "iii-mesh")
	p.Bin = filepath.Join(programs, "iii-mesh", "mesh.exe")
	p.Unit = windowsServiceName
	return p
}
func nativeInstall(source string) error {
	p := DefaultPaths()
	if p.State == "" || p.Bin == "" {
		return errors.New("Windows known folders unavailable")
	}
	m, err := mgr.Connect()
	if err != nil {
		return err
	}
	defer m.Disconnect()
	var dll, license []byte
	var service *mgr.Service
	defer func() {
		if service != nil {
			service.Close()
		}
	}()
	return installWindows(windowsInstallation{
		fetch: func() error { var err error; dll, license, err = downloadWintun(context.Background()); return err },
		stop:  func() error { return stopWindowsService(m, false) },
		place: func() error {
			if err := winsecurity.EnsureDirectory(p.State, false); err != nil {
				return err
			}
			if err := winsecurity.EnsureDirectory(filepath.Dir(p.Bin), true); err != nil {
				return err
			}
			if err := winsecurity.EnsureDirectory(p.Cache, true); err != nil {
				return err
			}
			if _, err := os.Lstat(p.Bin); err == nil {
				if err = winsecurity.Check(p.Bin, true); err != nil {
					return err
				}
			} else if !os.IsNotExist(err) {
				return err
			}
			if err := place(source, p.Bin); err != nil {
				return err
			}
			for name, b := range map[string][]byte{"wintun.dll": dll, "WINTUN-LICENSE.txt": license} {
				if err := publishWindowsFile(filepath.Join(filepath.Dir(p.Bin), name), b); err != nil {
					return err
				}
			}
			return winsecurity.Protect(p.Bin, true)
		},
		register: func() error {
			c := mgr.Config{ServiceType: windows.SERVICE_WIN32_OWN_PROCESS, StartType: mgr.StartAutomatic, ErrorControl: mgr.ErrorNormal, BinaryPathName: syscall.EscapeArg(p.Bin) + " node", DisplayName: "iii mesh node", Description: "iii mesh networking and operator workers", ServiceStartName: "LocalSystem"}
			var err error
			service, err = m.OpenService(windowsServiceName)
			if errors.Is(err, windows.ERROR_SERVICE_DOES_NOT_EXIST) {
				service, err = m.CreateService(windowsServiceName, p.Bin, c, "node")
			} else if err == nil {
				err = service.UpdateConfig(c)
			}
			if err != nil {
				return err
			}
			if err = service.SetRecoveryActions([]mgr.RecoveryAction{{Type: mgr.ServiceRestart, Delay: 2 * time.Second}}, 0); err != nil {
				return err
			}
			return service.SetRecoveryActionsOnNonCrashFailures(true)
		},
		start: func() error { return service.Start() },
	})
}
func publishWindowsFile(path string, b []byte) error {
	if err := winsecurity.Check(filepath.Dir(path), true); err != nil {
		return err
	}
	if _, err := os.Lstat(path); err == nil {
		if err = winsecurity.Check(path, true); err != nil {
			return err
		}
	} else if !os.IsNotExist(err) {
		return err
	}
	f, err := winsecurity.CreatePrivateTemp(filepath.Dir(path), ".install-")
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
	if err = winsecurity.Protect(f.Name(), true); err != nil {
		return err
	}
	return os.Rename(f.Name(), path)
}
func nativeUninstall() error {
	m, err := mgr.Connect()
	if err != nil {
		return err
	}
	defer m.Disconnect()
	return stopWindowsService(m, true)
}
func stopWindowsService(m *mgr.Mgr, remove bool) error {
	s, err := m.OpenService(windowsServiceName)
	if errors.Is(err, windows.ERROR_SERVICE_DOES_NOT_EXIST) {
		return nil
	}
	if err != nil {
		return err
	}
	defer s.Close()
	if remove {
		return uninstallWindows(s.ResetRecoveryActions, func() error { return stopServiceProcess(s) }, s.Delete)
	}
	return stopServiceProcess(s)
}
func stopServiceProcess(s *mgr.Service) error {
	st, err := s.Query()
	if err != nil {
		return err
	}
	if st.State != svc.Stopped {
		h, err := windows.OpenProcess(windows.SYNCHRONIZE, false, st.ProcessId)
		if err != nil {
			return err
		}
		defer windows.CloseHandle(h)
		if _, err = s.Control(svc.Stop); err != nil && !errors.Is(err, windows.ERROR_SERVICE_NOT_ACTIVE) {
			return err
		}
		result, err := windows.WaitForSingleObject(h, 30000)
		if err != nil {
			return err
		}
		if result != windows.WAIT_OBJECT_0 {
			return errors.New("Windows mesh service did not stop in 30 seconds")
		}
	}
	return nil
}

// VerifyWintunInstalled runs before any native adapter creation or DLL load.
func VerifyWintunInstalled() error {
	p := DefaultPaths()
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	if !stringsEqualPath(filepath.Dir(exe), filepath.Dir(p.Bin)) {
		return errors.New("run the installed mesh.exe for native Windows networking")
	}
	dll := filepath.Join(filepath.Dir(p.Bin), "wintun.dll")
	if err = winsecurity.Check(dll, true); err != nil {
		return err
	}
	b, err := os.ReadFile(dll)
	if err != nil {
		return err
	}
	return verifyWintun(b)
}
func stringsEqualPath(a, b string) bool {
	return strings.EqualFold(filepath.Clean(a), filepath.Clean(b))
}

func RunNode(ctx context.Context, run func(context.Context) error) error {
	isService, err := svc.IsWindowsService()
	if err != nil {
		return err
	}
	if !isService {
		return run(ctx)
	}
	p := DefaultPaths()
	if err = winsecurity.EnsureDirectory(p.State, false); err != nil {
		return err
	}
	path := filepath.Join(p.State, "node.log")
	if _, err = os.Lstat(path); err == nil {
		if err = winsecurity.Check(path, false); err != nil {
			return err
		}
	} else if !os.IsNotExist(err) {
		return err
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
	if err != nil {
		return err
	}
	defer f.Close()
	if err = windows.SetStdHandle(windows.STD_ERROR_HANDLE, windows.Handle(f.Fd())); err != nil {
		return err
	}
	os.Stdout, os.Stderr = f, f
	log.SetOutput(f)
	return svc.Run(windowsServiceName, windowsHandler{run: run})
}

type windowsHandler struct{ run func(context.Context) error }

func (s windowsHandler) Execute(_ []string, requests <-chan svc.ChangeRequest, status chan<- svc.Status) (bool, uint32) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	status <- svc.Status{State: svc.StartPending, WaitHint: 30000}
	done := make(chan error, 1)
	go func() { done <- s.run(ctx) }()
	status <- svc.Status{State: svc.Running, Accepts: svc.AcceptStop | svc.AcceptShutdown}
	for {
		select {
		case err := <-done:
			if err != nil {
				log.Print(err)
				return true, 1
			}
			return false, 0
		case r := <-requests:
			switch r.Cmd {
			case svc.Interrogate:
				status <- r.CurrentStatus
			case svc.Stop, svc.Shutdown:
				status <- svc.Status{State: svc.StopPending, WaitHint: 30000}
				cancel()
				select {
				case err := <-done:
					if err != nil && !errors.Is(err, context.Canceled) {
						log.Print(err)
						return true, 1
					}
					return false, 0
				case <-time.After(30 * time.Second):
					log.Print("Windows mesh shutdown timed out")
					return true, 1
				}
			}
		}
	}
}

package cli

import (
	"errors"
	"path/filepath"

	"github.com/iii-hq/workers/mesh/internal/service"
	"github.com/iii-hq/workers/mesh/internal/supervisor"
	"github.com/iii-hq/workers/mesh/internal/winsecurity"
)

func privateDir(dir string) error { return winsecurity.EnsureDirectory(dir, false) }
func checkPrivateState(dir, name string) error {
	if err := privateDir(dir); err != nil {
		return err
	}
	return winsecurity.Check(filepath.Join(dir, name), false)
}
func configurePlatform(a *app) *app {
	a.resolveOperator = func(account string) (string, string, error) {
		op, err := supervisor.LookupOperator(account)
		return op.Name(), op.ID(), err
	}
	up := a.up
	a.up = func(s State, dir string) (networkService, error) {
		if s.OperatorID == "" {
			return nil, errors.New("Windows state is missing the original operator SID; rejoin required")
		}
		if err := winsecurity.EnsureDirectory(filepath.Join(dir, "mesh"), false); err != nil {
			return nil, err
		}
		return up(s, dir)
	}
	return a
}
func removeInstalled(p service.Paths) error {
	var result error
	for _, path := range []string{p.Bin, p.Bin + ".previous", filepath.Join(filepath.Dir(p.Bin), "wintun.dll"), filepath.Join(filepath.Dir(p.Bin), "WINTUN-LICENSE.txt")} {
		result = errors.Join(result, winsecurity.RemoveFile(path))
	}
	if err := winsecurity.RemoveDirectory(filepath.Dir(p.Bin)); err != nil {
		result = errors.Join(result, err)
	}
	return result
}

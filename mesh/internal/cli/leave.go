package cli

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/iii-hq/workers/mesh/internal/elevate"
)

func Leave(ctx context.Context) error {
	if !elevate.Is() {
		return errors.New("mesh leave requires elevation")
	}
	return defaults("").leave(ctx)
}
func (a *app) leave(ctx context.Context) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	enrolled := exists(filepath.Join(a.paths.State, "join.json"))
	if enrolled {
		if err := writePrivate(a.paths.State, "leave-request", []byte("logout\n")); err != nil {
			return err
		}
	}
	if err := a.uninstall(); err != nil {
		return err
	}
	if enrolled && !removedStatus(a.paths.State) {
		result, err := os.ReadFile(filepath.Join(a.paths.State, "logout-result"))
		if err != nil {
			return fmt.Errorf("node service did not acknowledge logout; run the installed mesh node with elevation to complete logout, then retry mesh leave: %w", err)
		}
		if string(result) != "ok" {
			return fmt.Errorf("hub logout failed: %s; run the installed mesh node with elevation to complete logout, then retry mesh leave", result)
		}
	}
	if enrolled {
		if err := a.cleanupSavedDrives(); err != nil {
			return err
		}
	}
	cleanErr := a.clean()
	return errors.Join(cleanErr, os.RemoveAll(a.paths.Cache), os.RemoveAll(a.paths.State), removeInstalled(a.paths))
}
func removeIfExists(path string) error {
	err := os.Remove(path)
	if os.IsNotExist(err) {
		return nil
	}
	return err
}

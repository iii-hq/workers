package cli

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/iii-hq/workers/mesh/internal/node"
)

func (a *app) node(ctx context.Context) error {
	if removedStatus(a.paths.State) {
		if exists(filepath.Join(a.paths.State, "leave-request")) {
			return writePrivate(a.paths.State, "logout-result", []byte("ok"))
		}
		return a.park(ctx)
	}
	s, err := loadState(a.paths.State)
	if err != nil {
		return err
	}
	network, err := a.up(s, a.paths.State)
	if err != nil {
		return err
	}
	if exists(filepath.Join(a.paths.State, "leave-request")) {
		err := a.logout(ctx, network)
		return errors.Join(err, network.Close())
	}
	workers, err := a.workers(s)
	if err != nil {
		return errors.Join(err, network.Close())
	}
	runtime, err := a.newNode(node.Options{Mesh: network, Workers: workers, HubIdentity: s.Identity.Identity, Version: a.version, Operator: s.Operator, BinaryPath: a.paths.Bin})
	if err != nil {
		return errors.Join(err, workers.Close(), network.Close())
	}
	network.Report(func() string {
		var b strings.Builder
		fmt.Fprintf(&b, "Hub: %s\nOperator: %s\n", s.PublicURL, s.Operator)
		for _, w := range workers.Status() {
			fmt.Fprintf(&b, "Worker: %s %s %s\n", w.Name, w.Version, w.State)
		}
		return b.String()
	})
	runCtx, cancel := context.WithCancel(ctx)
	done := make(chan error, 1)
	go func() { done <- runtime.Run(runCtx) }()
	ticker := time.NewTicker(100 * time.Millisecond)
	persist := func() error {
		if name, ok := network.SelfName(); ok && name != s.Name {
			s.Name = name
			return saveState(a.paths.State, s)
		}
		return nil
	}
	var persistErr error
	if persistErr = persist(); persistErr != nil {
		cancel()
	}
	running := true
	for running {
		select {
		case err = <-done:
			running = false
		case <-ticker.C:
			if persistErr == nil {
				persistErr = persist()
				if persistErr != nil {
					cancel()
				}
			}
		}
	}
	cancel()
	ticker.Stop()
	closeErr := workers.Close()
	if exists(filepath.Join(a.paths.State, "leave-request")) {
		closeErr = errors.Join(closeErr, a.logout(ctx, network))
	}
	closeErr = errors.Join(closeErr, network.Close(), persistErr)
	if errors.Is(err, node.ErrRemoved) {
		if markerErr := writePrivate(a.paths.State, "removed", []byte("removed by hub\n")); markerErr != nil {
			return errors.Join(markerErr, closeErr)
		}
		if closeErr != nil {
			return closeErr
		}
		if err := a.park(ctx); err != nil {
			return err
		}
		if exists(filepath.Join(a.paths.State, "leave-request")) {
			return writePrivate(a.paths.State, "logout-result", []byte("ok"))
		}
		return nil
	}
	if closeErr == nil {
		return err
	}
	return errors.Join(err, closeErr)
}
func (a *app) logout(ctx context.Context, network networkService) error {
	if err := awaitEnrollment(ctx, network); errors.Is(err, node.ErrRemoved) {
		return writePrivate(a.paths.State, "logout-result", []byte("ok"))
	} else if err != nil {
		return err
	}
	err := network.Logout()
	result := "ok"
	if err != nil {
		result = err.Error()
	}
	return errors.Join(err, writePrivate(a.paths.State, "logout-result", []byte(result)))
}

func removedStatus(dir string) bool {
	for _, path := range []string{filepath.Join(dir, "removed"), filepath.Join(dir, "mesh", "removed")} {
		if _, err := os.Stat(path); err == nil {
			return true
		}
	}
	return false
}

func (a *app) park(ctx context.Context) error {
	text := "removed from the mesh by its hub; run mesh leave before joining again\n"
	if s, err := loadState(a.paths.State); err == nil {
		text += fmt.Sprintf("Hub: %s\nNode: %s\n", s.PublicURL, s.Name)
	}
	server, err := a.serveStatus(func() string { return text })
	if err != nil {
		return err
	}
	<-ctx.Done()
	return server.Close()
}

func awaitEnrollment(ctx context.Context, network networkService) error {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	for {
		select {
		case <-network.Removed():
			return node.ErrRemoved
		default:
		}
		if _, ok := network.SelfName(); ok {
			return nil
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("waiting for authenticated enrollment before logout: %w", ctx.Err())
		case <-network.Removed():
			return node.ErrRemoved
		case <-ticker.C:
		}
	}
}

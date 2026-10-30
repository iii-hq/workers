package cli

import (
	"context"
	"crypto/ed25519"
	"errors"
	"io"
	"os"
	"path/filepath"

	"github.com/iii-hq/workers/mesh/internal/elevate"
	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
	"github.com/iii-hq/workers/mesh/internal/service"
	"github.com/iii-hq/workers/mesh/internal/supervisor"
)

type networkService interface {
	node.Network
	Close() error
	Logout() error
	Report(func() string)
}
type workerService interface {
	node.WorkerController
	Close() error
}
type runner interface{ Run(context.Context) error }
type app struct {
	serveStatus          func(func() string) (io.Closer, error)
	paths                service.Paths
	version              string
	operator             func() (string, error)
	resolveOperator      func(string) (string, string, error)
	checkOperator        func(string) error
	hostname, executable func() (string, error)
	redeem               func(context.Context, string, string) (string, ed25519.PublicKey, error)
	identity             func(context.Context, string) (join.Identity, error)
	install              func(string, string) error
	uninstall, clean     func() error
	up                   func(State, string) (networkService, error)
	workers              func(State) (workerService, error)
	newNode              func(node.Options) (runner, error)
}
type ownedWorkers struct {
	node.WorkerController
	supervisor *supervisor.Supervisor
}

func (w ownedWorkers) Close() error { return w.supervisor.Close() }
func defaults(version string) *app {
	paths := service.DefaultPaths()
	return configurePlatform(&app{serveStatus: meshnet.ServeStatus, paths: paths, version: version, operator: elevate.Operator, checkOperator: func(name string) error { _, err := supervisor.LookupOperator(name); return err }, hostname: os.Hostname, executable: os.Executable, redeem: join.Redeem, identity: join.FetchIdentity, install: service.Install, uninstall: service.Uninstall, clean: meshnet.Clean,
		up: func(s State, dir string) (networkService, error) {
			return meshnet.UpWithPort(filepath.Join(dir, "mesh"), s.PublicURL, s.Secret, s.RequestedName, s.Identity.Control, uint16(s.Port))
		},
		workers: func(s State) (workerService, error) {
			op, err := supervisor.LookupOperator(s.operatorAccount())
			if err != nil {
				return nil, err
			}
			w := supervisor.New(paths.Cache, filepath.Join(paths.State, "logs"), op, "", "")
			return ownedWorkers{node.NewWorkerController(w), w}, nil
		},
		newNode: func(o node.Options) (runner, error) { return node.New(o) }})
}
func Execute(ctx context.Context, args []string, version string, w io.Writer) error {
	if len(args) == 0 {
		return errors.New("expected join, status, leave or node")
	}
	switch args[0] {
	case "join":
		a, err := ParseJoin(args[1:])
		if err != nil {
			return err
		}
		elevate.Root(args...)
		return Join(ctx, a.PublicURL, a.Code, a.Port)
	case "status":
		if len(args) != 1 {
			return errors.New("usage: mesh status")
		}
		return Status(w)
	case "leave":
		if len(args) != 1 {
			return errors.New("usage: mesh leave")
		}
		elevate.Root(args...)
		return Leave(ctx)
	case "node":
		if len(args) != 1 {
			return errors.New("usage: mesh node")
		}
		if !elevate.Is() {
			return errors.New("mesh node requires root")
		}
		return service.RunNode(ctx, defaults(version).node)
	default:
		return errors.New("unknown mesh command")
	}
}

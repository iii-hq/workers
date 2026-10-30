package node

import (
	"path/filepath"
	"runtime"

	"github.com/iii-hq/workers/mesh/internal/registry"
)

type Platform struct{ OS, Arch, Target string }

// A static mesh executable has no ELF interpreter. The musl loader identifies
// systems that need musl worker artifacts, including an unsupported arm64 host.
func HostPlatform() Platform {
	musl := false
	if runtime.GOOS == "linux" {
		loaders, _ := filepath.Glob("/lib/ld-musl-*.so.1")
		musl = len(loaders) > 0
	}
	return Platform{OS: runtime.GOOS, Arch: runtime.GOARCH, Target: registry.TargetFor(runtime.GOOS, runtime.GOARCH, musl)}
}

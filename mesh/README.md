# Mesh worker

The mesh Go worker currently registers `mesh::status` with an iii engine.
Status reports the build version; all other status fields are empty or zero
until the coordinator, networking and configuration are implemented. Joining,
leaving and node mode are not available yet.

Build and test with Go 1.27.1:

```sh
make build
make test
```

Start the worker with an engine running at the default URL:

```sh
./bin/mesh --url ws://127.0.0.1:49134
```

`III_URL` sets the default engine URL. `III_NAMESPACE` selects the registration
namespace and defaults to `default`. Stop the worker with Ctrl-C.

Interface capture uses registration only and initializes no external backend:

```sh
./bin/mesh --local-config --collect-interface
```

The public manifest declares `configuration` and `rbac-proxy` as runtime
dependencies. This scaffold does not call them yet. See `NOTICE` for upstream
source provenance and licenses.

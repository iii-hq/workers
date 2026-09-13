# Contributor checks

This is an original optional iii worker. Follow the repository's
[new-worker SOP](https://github.com/iii-hq/workers/blob/main/docs/sops/new-worker.md).

```bash
npm ci --ignore-scripts
npm test
npm run lint
```

The build emits typed library exports and a self-contained Node bundle at
`dist/bundle/index.mjs`. It uses the native SDK, not a substitute. Importing the
library is inert; `startWorker` explicitly connects and registers the two functions.
Only Guild HTTP is substituted in tests.

With the official iii 0.23.0 engine already installed and Python 3.11+ available:

```bash
III_BIN=/absolute/path/to/iii npm run test:e2e
```

The test starts its own loopback engine and HTTP fixture, captures the interface
with the repository's real collector, stages the bundle outside `node_modules`,
then invokes both functions across separate real SDK connections. The fixture
maps only the fixed Guild HTTP boundary to localhost. It never calls production,
uses a model, registers with Guild or makes a payment. Synthetic verifier flags
are not cryptographic verification. Results and logs are under
`tests/e2e/results/` (or `AGENT_GUILD_E2E_RESULTS`); all processes are shut down.
`PYTHON` can select a Python 3.11+ executable for the collector.

Run the native worker validator and descriptor compiler from repository root,
using the actual candidate commit SHA for the compiled index. Metadata checks
alone cannot prove runtime behavior. The dedicated workflow runs all these
checks and the engine protocol test on Node 22. Maintainers retain release and
registry publication authority.

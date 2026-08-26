/**
 * The entry `iii.worker.yaml` names: `scripts.start` is `node ./index.mjs`.
 *
 * In a published bundle that path IS the bundle (`dist/bundle/index.mjs` ships
 * as the package root). From a source checkout the same command lands here, and
 * here it forwards to the compiled output — so a supervisor that reads the
 * manifest starts this worker either way, and a `worker-compose.yaml` needs no
 * `run` line of its own. Build first: `pnpm install && pnpm run build`.
 */

import './dist/index.js';

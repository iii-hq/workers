/**
 * Composition root. Tests call this with a fake forwardTrigger instead of
 * reaching into a mutable module bag. Caches live on `deps` so they die
 * with the runtime, not as process globals.
 */
import path from 'node:path';

import { createMiddlewareHandler } from './middleware.js';
import { resolveLeaseStorePath } from './repo-path.js';
import { BoundedMap, getLeaseStore } from './stores.js';
import { createVerifyHandler, onVerifyPassed, VerifyCoalescer } from './verify.js';

export function createGantryRuntime({
  forwardTrigger,
  verdictKeyringPath,
  leaseStorePathOverride,
  emitVerdict,
} = {}) {
  if (typeof forwardTrigger !== 'function') {
    throw new TypeError('opengantry: forwardTrigger is required');
  }
  const envVerdictKeyring = process.env.GANTRY_VERDICT_KEYRING?.trim() || undefined;
  const envLeaseStoreOverride = process.env.GANTRY_III_LEASE_STORE?.trim() || undefined;
  const deps = {
    forwardTrigger,
    leaseStores: new BoundedMap(32),
    governance: new BoundedMap(32),
    coalescer: new VerifyCoalescer(),
    verdictKeyringPath: verdictKeyringPath ?? envVerdictKeyring,
    leaseStorePathOverride: leaseStorePathOverride ?? envLeaseStoreOverride,
    resolveVerdictKeyringPath(repoRoot) {
      if (this.verdictKeyringPath) return this.verdictKeyringPath;
      return path.join(repoRoot, '.config/gantry/pepper-keyring.json');
    },
    resolveLeaseStorePath(repoRoot) {
      return resolveLeaseStorePath(repoRoot, this.leaseStorePathOverride);
    },
    emitVerdict: emitVerdict ?? (async () => {}),
  };
  return {
    middleware: createMiddlewareHandler(deps),
    verify: createVerifyHandler(deps),
    onVerifyPassed: (data) => onVerifyPassed(deps, data),
    leaseStoreFor: (repoRoot) => getLeaseStore(deps, repoRoot),
  };
}

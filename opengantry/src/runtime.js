import { createMiddlewareHandler } from './middleware.js';
import { BoundedMap, getLeaseStore } from './stores.js';
import { createVerifyHandler, onVerifyPassed, VerifyCoalescer } from './verify.js';

export function createGantryRuntime({ forwardTrigger }) {
  if (typeof forwardTrigger !== 'function') {
    throw new TypeError('opengantry: forwardTrigger is required');
  }
  const deps = {
    forwardTrigger,
    leaseStores: new BoundedMap(32),
    governance: new BoundedMap(32),
    coalescer: new VerifyCoalescer(),
  };
  return {
    middleware: createMiddlewareHandler(deps),
    verify: createVerifyHandler(deps),
    onVerifyPassed: (data) => onVerifyPassed(deps, data),
    leaseStoreFor: (repoRoot) => getLeaseStore(deps, repoRoot),
  };
}

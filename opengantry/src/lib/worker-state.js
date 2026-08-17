import { VerifyCoalescer } from './verify-coalescer.js';

export function createWorkerState() {
  return {
    leaseStores: undefined,
    governance: undefined,
    coalescer: new VerifyCoalescer(),
    forwardTrigger: async (function_id, payload) => ({ ok: true, function_id, payload }),
  };
}

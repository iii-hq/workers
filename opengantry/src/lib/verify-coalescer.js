/** Single-flight verify coalescing keyed by repo root. */
export class VerifyCoalescer {
  constructor() {
    this.inFlight = new Map();
    this.queueDepth = 0;
    this.maxQueue = 32;
  }

  async run(key, fn) {
    if (this.inFlight.has(key)) {
      return this.inFlight.get(key);
    }
    if (this.queueDepth >= this.maxQueue) {
      return {
        status: 'failed',
        error_code: 'GXT_VERIFY_SATURATED',
        findings: [
          {
            failed_gate: 'gate',
            resolution_hint: 'verify queue saturated; retry later',
          },
        ],
      };
    }
    this.queueDepth += 1;
    const promise = fn().finally(() => {
      this.inFlight.delete(key);
      this.queueDepth -= 1;
    });
    this.inFlight.set(key, promise);
    return promise;
  }
}

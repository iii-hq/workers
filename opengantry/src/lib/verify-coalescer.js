/** Single-flight verify coalescing keyed by caller-supplied cache key. */
export class VerifyCoalescer {
  constructor() {
    this.inFlight = new Map();
    this.maxQueue = 32;
  }

  async run(key, fn) {
    if (this.inFlight.has(key)) {
      return this.inFlight.get(key);
    }
    if (this.inFlight.size >= this.maxQueue) {
      return {
        status: 'failed',
        error_code: 'GXT_VERIFY_SATURATED',
      };
    }
    const promise = fn().finally(() => {
      this.inFlight.delete(key);
    });
    this.inFlight.set(key, promise);
    return promise;
  }
}

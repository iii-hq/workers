/**
 * Governance denial. Throw so iii records InvocationResult.error.
 * Returning { ok: false } would look like a successful trigger.
 */
export class GantryDenied extends Error {
  constructor(code, hint) {
    super(`[${code}] ${hint}`);
    this.name = 'GantryDenied';
    this.code = code;
    this.hint = hint;
  }
}

/** Governance denial surfaced to iii as InvocationResult.error via throw. */
export class GantryDenied extends Error {
  constructor(code, hint) {
    super(`[${code}] ${hint}`);
    this.name = 'GantryDenied';
    this.code = code;
    this.hint = hint;
  }
}

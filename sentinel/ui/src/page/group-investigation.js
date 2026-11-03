/** Evidence excerpt and request-body budgets; JSON envelopes also consume context. */
export const MEMBER_EVIDENCE_MAX_BYTES = 3500
export const CONTEXT_MAX_BYTES = 32768
/** UTF-8, not UTF-16: the backend limit is in bytes. */
export const hypothesisBytes = (/** @type {string} */ value) => new TextEncoder().encode(value).length
export function groupInvestigationProblem(/** @type {string[]} */ ids, /** @type {string} */ hypothesis = '') {
  if (ids.length < 2 || ids.length > 5) return 'Select 2–5 issues for one investigation.'
  if (new Set(ids).size !== ids.length || ids.some(id => !id)) return 'Select distinct issues.'
  if (hypothesisBytes(hypothesis) > 2000) return 'The optional hypothesis must fit within 2000 UTF-8 bytes.'
  return ''
}
/** Reuse the key after an ambiguous transport failure; new intent gets a new key. */
export function intentSignature(/** @type {string[]} */ ids, /** @type {string} */ profile, /** @type {string} */ hypothesis) {
  return JSON.stringify([[...ids].sort(), profile, hypothesis])
}

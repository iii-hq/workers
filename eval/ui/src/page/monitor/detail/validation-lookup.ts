// Pure logic of the "Attach E2E runs" dialog: what the typed ids allow, how a
// rejected `eval::attach-validation` call is read, and which footer action
// the current lookup leaves. No React, no host — unit-tested.
import type { ValidationLink } from '../../../types'

export const SAME_ID_MESSAGE = 'Baseline and candidate must be different executions.'
export const NOT_FOUND_MESSAGE = 'Not found in the E2E service. Check the ID; the execution may have been deleted.'

export type IdState = 'incomplete' | 'same' | 'ready'

/** What the two typed ids allow before any lookup: both filled and different. */
export function idState(baseline: string, candidate: string): IdState {
  const b = baseline.trim()
  const c = candidate.trim()
  if (!b || !c) return 'incomplete'
  return b === c ? 'same' : 'ready'
}

/** One key per pair of ids; a lookup result is only valid for its own key. */
export function lookupKey(baseline: string, candidate: string): string {
  return `${baseline.trim()}\u0000${candidate.trim()}`
}

export type Side = 'baseline' | 'candidate'

export type AttachFailure =
  /**
   * The E2E service answered but has no such execution. `fields` are the
   * sides the backend named; `ids` the id a free-text message carried, for
   * a backend that names no side.
   */
  | { kind: 'not_found'; fields: Side[]; ids: string[]; message: string }
  /** The E2E worker did not answer: nothing was checked or saved. */
  | { kind: 'unavailable'; message: string }
  /** Anything else (the analysis is not finished, the suggestion is gone…). */
  | { kind: 'failed'; message: string }

// `eval::attach-validation` words its errors with stable codes:
// `e2e_execution_not_found(baseline|candidate)` when that execution does not
// exist (or its id is invalid) and `e2e_unavailable` when the E2E service
// cannot be reached. They are read first; the wording heuristic below is only
// for a backend that does not send them.
const NOT_FOUND_CODE = /e2e_execution_not_found\((baseline|candidate)\)/g
const UNAVAILABLE_CODE = 'e2e_unavailable'

// Without a code the backend words a failed read as `E2E execution <id> could
// not be read: <reason>`, and the reason always carries the id of the function
// it called (`dependency error: e2e::dashboard::execution-get failed: …`), so
// that id says nothing about the service. The reason of a worker that is not
// there at all (function not found, timeout, bus) says the service is down;
// any other reason means that execution does not exist for the service.
const UNREAD = /E2E execution (\S+) could not be read:\s*([\s\S]*)$/
const SERVICE_DOWN =
  /function[\s_]not[\s_]found|function[^\n]{0,80}not found|not found[^\n]{0,40}function|time(?:d)?[ -]?out|unavailable|not registered|not connected|disconnect|connection|refused|\bbus\b|offline|no worker|worker/i

export function classifyAttachFailure(message: string): AttachFailure {
  if (message.includes(UNAVAILABLE_CODE)) return { kind: 'unavailable', message }
  const fields = [...new Set([...message.matchAll(NOT_FOUND_CODE)].map((match) => match[1] as Side))]
  if (fields.length > 0) return { kind: 'not_found', fields, ids: [], message }
  const unread = UNREAD.exec(message)
  if (unread) {
    return SERVICE_DOWN.test(unread[2])
      ? { kind: 'unavailable', message }
      : { kind: 'not_found', fields: [], ids: [unread[1]], message }
  }
  if (SERVICE_DOWN.test(message)) return { kind: 'unavailable', message }
  return { kind: 'failed', message }
}

/** What the dialog shows for the ids currently typed. */
export type LookupView =
  | { phase: 'idle' }
  | { phase: 'checking' }
  | { phase: 'found'; link: ValidationLink }
  | { phase: 'failed'; failure: AttachFailure }

export type PrimaryKind = 'attach' | 'attach_anyway' | 'retry'

/**
 * The one action of the footer. Attach needs both runs found and different;
 * runs that are not comparable are still allowed, as "Attach anyway"; a
 * service that did not answer offers "Try again" instead, also when it is the
 * list of runs to pick from (`listFailed`) that it did not give.
 */
export function primaryAction(
  view: LookupView,
  ids: IdState,
  listFailed = false,
): { kind: PrimaryKind; enabled: boolean } {
  if (listFailed) return { kind: 'retry', enabled: true }
  if (view.phase === 'failed' && view.failure.kind !== 'not_found') return { kind: 'retry', enabled: true }
  if (view.phase === 'found') {
    return { kind: view.link.comparability.comparable ? 'attach' : 'attach_anyway', enabled: ids === 'ready' }
  }
  return { kind: 'attach', enabled: false }
}

/** Whether a not-found failure belongs to this field: the side it names, else the id it carries. */
export function failedField(failure: AttachFailure | undefined, field: Side, ids: Record<Side, string>): boolean {
  return failure?.kind === 'not_found' && (failure.fields.includes(field) || failure.ids.includes(ids[field].trim()))
}

/** @typedef {{ pending: boolean, error: string | null }} RunRetryState */
/** @typedef {Record<string, RunRetryState>} RunRetryStates */
/** @typedef {{ kind: 'run', runId: string } | { kind: 'filter' } | null} FocusTarget */

/**
 * @param {RunRetryStates} states
 * @param {string} runId
 * @returns {RunRetryStates}
 */
export function beginRetry(states, runId) {
  return { ...states, [runId]: { pending: true, error: null } }
}

/**
 * @param {RunRetryStates} states
 * @param {string} runId
 * @param {string | null} error
 * @returns {RunRetryStates}
 */
export function settleRetry(states, runId, error) {
  return { ...states, [runId]: { pending: false, error } }
}

/** @param {boolean} bound @param {unknown} connectionState */
export function isStreamLive(bound, connectionState) {
  return bound && connectionState === 'connected'
}

/**
 * A repository-scoped value is renderable only after that exact repository
 * filter has resolved. `null` represents the initial unresolved list.
 *
 * @param {string} activeRepository
 * @param {string | null} resolvedRepository
 */
export function isRepositoryScopeCurrent(activeRepository, resolvedRepository) {
  return resolvedRepository !== null && activeRepository === resolvedRepository
}

/**
 * @param {boolean} narrow
 * @param {boolean} detailOpen
 * @param {string | null} nextRunId
 * @returns {FocusTarget}
 */
export function automaticFocusTarget(narrow, detailOpen, nextRunId) {
  if (!narrow || !detailOpen) return null
  return nextRunId ? { kind: 'run', runId: nextRunId } : { kind: 'filter' }
}

/** @param {number} visible @param {number} total @param {number} step */
export function nextVisibleFindingCount(visible, total, step) {
  return Math.min(total, visible + step)
}

/**
 * @param {number} previousRevision
 * @param {number} nextRevision
 * @param {string | null} runId
 */
export function shouldReloadReconciliation(
  previousRevision,
  nextRevision,
  runId,
) {
  return Boolean(runId) && previousRevision !== nextRevision
}

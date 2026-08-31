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

/** @param {number} total @param {boolean} narrow */
export function scanHistoryDescription(total, narrow) {
  if (!narrow) return `${total} recent repository reviews`
  return `${total} ${total === 1 ? 'run' : 'runs'}`
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

export const ANALYSIS_SESSION_PREFIX = 'security-scan-analysis-'
export const ANALYSIS_SESSION_TITLE = 'Security review'

/** @param {unknown} value */
export function analysisConversationFromSession(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const item = /** @type {Record<string, unknown>} */ (value)
  const sessionId = typeof item.session_id === 'string' ? item.session_id.trim() : ''
  const title = typeof item.title === 'string' ? item.title.trim() : ''
  const metadata =
    item.metadata && typeof item.metadata === 'object' && !Array.isArray(item.metadata)
      ? /** @type {Record<string, unknown>} */ (item.metadata)
      : null
  const runId =
    typeof metadata?.security_scan_run_id === 'string'
      ? metadata.security_scan_run_id.trim()
      : ''
  if (
    !sessionId.startsWith(ANALYSIS_SESSION_PREFIX) ||
    title !== ANALYSIS_SESSION_TITLE ||
    metadata?.security_scan !== true ||
    !runId
  ) {
    return null
  }
  return { sessionId, runId }
}

/**
 * @param {{
 *   followRunId: string | null,
 *   startConversationId?: string | null,
 *   currentConversationId?: string | null,
 * }} input
 */
export function shouldFollowAnalysisChat(input) {
  if (!input.followRunId) return false
  const current = input.currentConversationId?.trim() ?? ''
  const start = input.startConversationId?.trim() ?? ''
  if (!current || !start) return true
  if (current === start) return true
  return current.startsWith(ANALYSIS_SESSION_PREFIX)
}

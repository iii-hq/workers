/**
 * The security-scan worker's own change trigger types. Each fires after the
 * worker committed a change to a stored record and carries a small
 * notification (record id plus its `updated_at` revision hint), never the
 * record: the page re-reads the record through the worker's read functions.
 */
export const RUN_CHANGED_TRIGGER = 'security-scan::run-changed'
export const RECONCILIATION_CHANGED_TRIGGER = 'security-scan::reconciliation-changed'
export const ACTION_CHANGED_TRIGGER = 'security-scan::action-changed'

/** Tab-scoped handler for the runs feed; the host appends `::<browserId>`. */
export const RUNS_HANDLER_ID = 'iii::security-scan-ui::runs'

/**
 * What the runs page binds: run changes re-read the list (and with it the
 * selected run), reconciliation changes re-read the open run's alert
 * reconciliation. Unfiltered, like the single feed it replaces.
 */
export const RUN_LIVE_TRIGGERS = Object.freeze([RUN_CHANGED_TRIGGER, RECONCILIATION_CHANGED_TRIGGER])

/** @param {boolean} bound @param {unknown} connectionState */
export function isLiveUpdates(bound, connectionState) {
  return bound && connectionState === 'connected'
}

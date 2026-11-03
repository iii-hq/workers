/**
 * The security-scan worker's own change functions. The worker calls each one
 * after it committed a change to a stored record, with a small event (record
 * id plus its `updated_at` revision hint), never the record: the page
 * observes them and re-reads the record through the worker's read functions.
 */
export const ON_RUN_CHANGED_FN = 'security-scan::on-run-changed'
export const ON_RECONCILIATION_CHANGED_FN = 'security-scan::on-reconciliation-changed'
export const ON_ACTION_CHANGED_FN = 'security-scan::on-action-changed'

/** Tab-scoped handler for the runs feed; the host appends `::<browserId>`. */
export const RUNS_HANDLER_ID = 'iii::security-scan-ui::runs'

/**
 * What the runs page observes: run changes re-read the list (and with it the
 * selected run), reconciliation changes re-read the open run's alert
 * reconciliation. Unfiltered, so the page sees every run.
 */
export const RUN_LIVE_SOURCES = Object.freeze([ON_RUN_CHANGED_FN, ON_RECONCILIATION_CHANGED_FN])

/** @param {boolean} observing @param {unknown} connectionState */
export function isLiveUpdates(observing, connectionState) {
  return observing && connectionState === 'connected'
}

/** @typedef {import('@iii-dev/console-ui').Host} Host */
/** @typedef {import('./security-scan-data').ActionKind} ActionKind */
/** @typedef {import('./security-scan-data').ActionRequestResult} ActionRequestResult */
/** @typedef {import('./security-scan-data').SecurityAction} SecurityAction */

import { ACTION_CHANGED_TRIGGER } from './live-triggers.js'

/**
 * @typedef {{
 *   submitting: boolean,
 *   request: ActionRequestResult | null,
 *   action: SecurityAction | null,
 *   error: string | null,
 * }} FindingActionState
 */

/** @typedef {Record<string, FindingActionState>} SecurityActionsSnapshot */
/** @typedef {{ actionId: string, status: import('./security-scan-data').ActionStatus, updatedAt: number }} ActionUpdate */

/** @param {string} runId @param {number} findingIndex @param {ActionKind} action */
export function securityActionKey(runId, findingIndex, action) {
  return `${runId}\u0001${findingIndex}\u0001${action}`
}

/** @param {unknown} value */
function objectRecord(value) {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? /** @type {Record<string, unknown>} */ (value)
    : null
}

/**
 * Parses a `security-scan::action-changed` notification
 * (`{ action_id, run_id, repository, status, updated_at }`). Extra fields
 * the engine adds are ignored; anything else is not an action change.
 *
 * @param {unknown} payload
 * @returns {ActionUpdate | null}
 */
export function actionUpdateFromEvent(payload) {
  const data = objectRecord(payload)
  if (
    typeof data?.action_id !== 'string' ||
    !data.action_id ||
    typeof data.status !== 'string' ||
    typeof data.updated_at !== 'number'
  ) {
    return null
  }
  return {
    actionId: data.action_id,
    status: /** @type {import('./security-scan-data').ActionStatus} */ (data.status),
    updatedAt: data.updated_at,
  }
}

/**
 * @param {{
 *   host: Host,
 *   bindingId: string,
 *   requestAction(host: Host, runId: string, findingIndex: number, action: ActionKind): Promise<ActionRequestResult>,
 *   readAction(host: Host, actionId: string): Promise<SecurityAction | null>,
 *   errorText(error: unknown): string,
 * }} dependencies
 */
export function createSecurityActionsStore(dependencies) {
  const { host, bindingId, requestAction, readAction, errorText } = dependencies

  /** @type {SecurityActionsSnapshot} */
  let snapshot = {}
  /** @type {Set<() => void>} */
  const listeners = new Set()
  /** @type {Map<string, string>} */
  const keyByActionId = new Map()
  /** Highest notification revision seen per action id. @type {Map<string, number>} */
  const lastEventAt = new Map()
  /**
   * One authoritative read in flight per action id, plus at most one
   * pending re-read: duplicate notifications coalesce, and reads apply in
   * the order they were issued.
   * @type {Map<string, { again: boolean, done: Promise<boolean> }>}
   */
  const reads = new Map()
  /** @type {Array<() => void>} */
  let disposers = []
  let connected = false
  let bound = false
  let disposed = false

  const emit = () => {
    for (const listener of listeners) listener()
  }

  /** @param {string} key @param {(current: FindingActionState) => FindingActionState} update */
  const updateState = (key, update) => {
    const current = snapshot[key] ?? {
      submitting: false,
      request: null,
      action: null,
      error: null,
    }
    snapshot = { ...snapshot, [key]: update(current) }
    emit()
  }

  /** @param {string} actionId */
  async function readOnce(actionId) {
    const key = keyByActionId.get(actionId)
    if (!key || disposed) return false
    try {
      const action = await readAction(host, actionId)
      if (!action || disposed || keyByActionId.get(actionId) !== key) {
        return false
      }
      updateState(key, (current) => ({
        ...current,
        request: null,
        action,
        error: null,
      }))
      return true
    } catch {
      return false
    }
  }

  /**
   * Re-reads one tracked action. While a read for it is in flight, further
   * calls only mark it dirty, so a burst of notifications costs at most one
   * extra read and a slower earlier read can never land after a later one.
   *
   * @param {string} actionId
   * @returns {Promise<boolean>} whether a read result was applied
   */
  function refreshAction(actionId) {
    const running = reads.get(actionId)
    if (running) {
      running.again = true
      return running.done
    }
    const entry = { again: false, done: Promise.resolve(false) }
    reads.set(actionId, entry)
    entry.done = (async () => {
      let applied = false
      try {
        do {
          entry.again = false
          applied = (await readOnce(actionId)) || applied
        } while (entry.again && !disposed)
      } finally {
        reads.delete(actionId)
      }
      return applied
    })()
    return entry.done
  }

  /** @param {ActionUpdate} update */
  const applyUpdate = (update) => {
    const key = keyByActionId.get(update.actionId)
    if (!key) return
    const seen = lastEventAt.get(update.actionId)
    // A notification older than one already seen must not roll the
    // optimistic status back; the authoritative re-read still runs.
    if (seen === undefined || update.updatedAt >= seen) {
      lastEventAt.set(update.actionId, update.updatedAt)
      updateState(key, (current) => ({
        ...current,
        request:
          current.request?.action_id === update.actionId
            ? { ...current.request, status: update.status }
            : current.request,
      }))
    }
    void refreshAction(update.actionId)
  }

  const start = () => {
    disposed = false
    const handlerId = `iii::security-scan-ui::actions::${bindingId}`
    try {
      disposers.push(
        host.iii.on(handlerId, (payload) => {
          const update = actionUpdateFromEvent(payload)
          if (update) applyUpdate(update)
        }),
      )
      disposers.push(
        host.iii.registerTrigger({
          type: ACTION_CHANGED_TRIGGER,
          function_id: `${handlerId}::${host.iii.browserId}`,
          config: {},
        }),
      )
      bound = true
    } catch {
      for (const dispose of disposers) dispose()
      disposers = []
      bound = false
    }
    try {
      disposers.push(
        host.iii.addConnectionStateListener((state) => {
          connected = bound && state === 'connected'
          // Reconnect is the recovery path: notifications missed while the
          // socket was down are not replayed, so tracked actions are re-read
          // once, here.
          if (!connected) return
          for (const actionId of keyByActionId.keys()) {
            void refreshAction(actionId)
          }
        }),
      )
    } catch {
      connected = false
    }
    return dispose
  }

  /** @param {string} runId @param {number} findingIndex @param {ActionKind} action */
  const request = async (runId, findingIndex, action) => {
    const key = securityActionKey(runId, findingIndex, action)
    updateState(key, (current) => ({
      ...current,
      submitting: true,
      error: null,
    }))
    try {
      const response = await requestAction(host, runId, findingIndex, action)
      keyByActionId.set(response.action_id, key)
      updateState(key, (current) => ({
        ...current,
        submitting: false,
        request: response,
        action: current.action?.action_id === response.action_id ? current.action : null,
        error: null,
      }))
      // Initial read: also the only read when live updates are unavailable.
      await refreshAction(response.action_id)
    } catch (error) {
      updateState(key, (current) => ({
        ...current,
        submitting: false,
        error: errorText(error),
      }))
    }
  }

  function dispose() {
    if (disposed) return
    disposed = true
    for (const disposer of disposers.reverse()) disposer()
    disposers = []
    listeners.clear()
  }

  return {
    getSnapshot: () => snapshot,
    /** @param {() => void} listener */
    subscribe(listener) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    start,
    request,
    dispose,
  }
}

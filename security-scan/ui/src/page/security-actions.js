/** @typedef {import('@iii-dev/console-ui').Host} Host */
/** @typedef {import('./security-scan-data').ActionKind} ActionKind */
/** @typedef {import('./security-scan-data').ActionRequestResult} ActionRequestResult */
/** @typedef {import('./security-scan-data').SecurityAction} SecurityAction */

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

const ACTION_EVENT_TYPE = 'security-scan:action-updated'
const ACTION_STREAM_NAME = 'security-scan:runs'
const ACTION_STREAM_GROUP = 'all'

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

/** @param {unknown} frame @returns {ActionUpdate | null} */
export function actionUpdateFromFrame(frame) {
  const root = objectRecord(frame)
  const outer = objectRecord(root?.event)
  const inner = objectRecord(outer?.event) ?? outer ?? root
  if (inner?.type !== ACTION_EVENT_TYPE) return null
  const data = objectRecord(inner.data)
  if (
    typeof data?.action_id !== 'string' ||
    typeof data.status !== 'string' ||
    typeof data.updated_at !== 'number'
  ) {
    return null
  }
  return {
    actionId: data.action_id,
    status: /** @type {import('./security-scan-data').ActionStatus} */ (
      data.status
    ),
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
  const {
    host,
    bindingId,
    requestAction,
    readAction,
    errorText,
  } = dependencies

  /** @type {SecurityActionsSnapshot} */
  let snapshot = {}
  /** @type {Set<() => void>} */
  const listeners = new Set()
  /** @type {Map<string, string>} */
  const keyByActionId = new Map()
  /** @type {Array<() => void>} */
  let disposers = []
  let connected = false
  let streamBound = false
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
  async function refreshAction(actionId) {
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

  /** @param {ActionUpdate} update */
  const applyUpdate = (update) => {
    const key = keyByActionId.get(update.actionId)
    if (!key) return
    updateState(key, (current) => ({
      ...current,
      request:
        current.request?.action_id === update.actionId
          ? { ...current.request, status: update.status }
          : current.request,
    }))
    void refreshAction(update.actionId)
  }

  const start = () => {
    disposed = false
    const handlerId = `iii::security-scan-ui::actions::${bindingId}`
    try {
      disposers.push(
        host.iii.on(handlerId, (frame) => {
          const update = actionUpdateFromFrame(frame)
          if (update) applyUpdate(update)
        }),
      )
      disposers.push(
        host.iii.registerTrigger({
          type: 'stream',
          function_id: `${handlerId}::${host.iii.browserId}`,
          config: {
            stream_name: ACTION_STREAM_NAME,
            group_id: ACTION_STREAM_GROUP,
          },
        }),
      )
      streamBound = true
    } catch {
      for (const dispose of disposers) dispose()
      disposers = []
      streamBound = false
    }
    try {
      disposers.push(
        host.iii.addConnectionStateListener((state) => {
          connected = streamBound && state === 'connected'
          // Reconnect is the recovery path: frames missed while the socket
          // was down are re-read once, here.
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
      const response = await requestAction(
        host,
        runId,
        findingIndex,
        action,
      )
      keyByActionId.set(response.action_id, key)
      updateState(key, (current) => ({
        ...current,
        submitting: false,
        request: response,
        action:
          current.action?.action_id === response.action_id
            ? current.action
            : null,
        error: null,
      }))
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

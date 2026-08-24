import type { Host } from '@iii-dev/console-ui'
import { useEffect, useId, useMemo, useSyncExternalStore } from 'react'
import { errText } from './errors.js'
import {
  createSecurityActionsStore,
  securityActionKey,
} from './security-actions.js'
import {
  type ActionKind,
  type ActionRequestResult,
  readFindingAction,
  requestFindingAction,
  type SecurityAction,
} from './security-scan-data'

export interface FindingActionState {
  submitting: boolean
  request: ActionRequestResult | null
  action: SecurityAction | null
  error: string | null
}

const EMPTY_ACTION_STATE: FindingActionState = {
  submitting: false,
  request: null,
  action: null,
  error: null,
}

export interface SecurityActionsLive {
  stateFor(
    runId: string,
    findingIndex: number,
    action: ActionKind,
  ): FindingActionState
  request(
    runId: string,
    findingIndex: number,
    action: ActionKind,
  ): Promise<void>
}

export function useSecurityActions(host: Host): SecurityActionsLive {
  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')
  const store = useMemo(
    () =>
      createSecurityActionsStore({
        host,
        bindingId: instanceId,
        requestAction: requestFindingAction,
        readAction: readFindingAction,
        errorText: errText,
      }),
    [host, instanceId],
  )

  useEffect(() => store.start(), [store])

  const snapshot = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  )

  return useMemo(
    () => ({
      stateFor(runId, findingIndex, action) {
        return (
          snapshot[securityActionKey(runId, findingIndex, action)] ??
          EMPTY_ACTION_STATE
        )
      },
      request: store.request,
    }),
    [snapshot, store],
  )
}

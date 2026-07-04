/**
 * Live subscription to `harness::comm` for one session family, plus the
 * one-shot history fetch. Same two-step binding pattern as
 * `turn-events-live.ts`; the engine evaluates the `{ root_session_id }`
 * filter so a browser only sees its own family's edges.
 */

import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import type { CommEvent, CommHistoryResponse } from '@/types/iii-agent-event'

const COMM_FN = 'iii::console::comm'
const COMM_TRIGGER = 'harness::comm'

type ClientSubset = Pick<IiiClient, 'browserId' | 'on' | 'registerTrigger'>

export function startCommEventsSubscription(
  client: ClientSubset,
  rootSessionId: string,
  onEvent: (event: CommEvent) => void,
): () => void {
  const off = client.on(COMM_FN, (payload: unknown) => {
    if (payload && typeof payload === 'object') onEvent(payload as CommEvent)
  })
  const offTrigger = client.registerTrigger({
    type: COMM_TRIGGER,
    function_id: `${COMM_FN}::${client.browserId}`,
    config: { root_session_id: rootSessionId },
  })
  return () => {
    off()
    try {
      offTrigger()
    } catch {
      // SDK already disposed; nothing to do.
    }
  }
}

export async function fetchCommHistory(
  rootSessionId: string,
): Promise<CommHistoryResponse> {
  const client = await getIiiClient()
  return client.trigger<CommHistoryResponse>('harness::comm::history', {
    root_session_id: rootSessionId,
  })
}

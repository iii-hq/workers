import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'
import {
  analysisConversationRunId,
  ensureAnalysisConversation,
  isSecurityAnalysisSession,
  openAnalysisConversation,
} from './security-scan-data'
import { shouldFollowAnalysisChat } from './view-state.js'

const FOLLOW_FN = 'security-scan-ui::follow-analysis'

export { shouldFollowAnalysisChat }

export function useFollowAnalysisChat(
  host: Host,
  followRunId: string | null,
  startConversationId: string | null | undefined,
  currentConversationId: string | null | undefined,
  onFollowed: () => void,
): void {
  const followRunIdRef = useRef(followRunId)
  const startConversationIdRef = useRef(startConversationId)
  const currentConversationIdRef = useRef(currentConversationId)
  const onFollowedRef = useRef(onFollowed)
  followRunIdRef.current = followRunId
  startConversationIdRef.current = startConversationId
  currentConversationIdRef.current = currentConversationId
  onFollowedRef.current = onFollowed

  useEffect(() => {
    const offHandler = host.iii.on<{ session_id?: string; title?: string }>(FOLLOW_FN, async (event) => {
      const runId = followRunIdRef.current
      if (
        !shouldFollowAnalysisChat({
          followRunId: runId,
          startConversationId: startConversationIdRef.current,
          currentConversationId: currentConversationIdRef.current,
        })
      ) {
        return
      }
      if (!runId || !isSecurityAnalysisSession(event) || !event.session_id) return
      if ((await analysisConversationRunId(host, event.session_id)) !== runId) return
      if (!openAnalysisConversation(host, event.session_id)) return
      onFollowedRef.current()
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'session::created',
      function_id: `${FOLLOW_FN}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host])

  // One attempt for a conversation that already exists; a conversation created
  // later arrives on the `session::created` binding above. No retry timer:
  // the event is the only other thing that can change the answer.
  useEffect(() => {
    if (!followRunId) return
    let cancelled = false
    void (async () => {
      if (
        !shouldFollowAnalysisChat({
          followRunId,
          startConversationId: startConversationIdRef.current,
          currentConversationId: currentConversationIdRef.current,
        })
      ) {
        return
      }
      try {
        const sessionId = await ensureAnalysisConversation(host, followRunId)
        if (cancelled || !sessionId) return
        if (openAnalysisConversation(host, sessionId)) onFollowedRef.current()
      } catch {
        // The event path remains; a transient read failure is not fatal.
      }
    })()
    return () => {
      cancelled = true
    }
  }, [followRunId, host])
}

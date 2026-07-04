import { useEffect, useMemo, useState } from 'react'
import { useTimelineRoute } from '@/hooks/use-hash-route'
import {
  fetchCommHistory,
  startCommEventsSubscription,
} from '@/lib/backend/comm-events-live'
import { buildLanes, mergeEvents, resolveRootId } from '@/lib/comm/timeline'
import { useConversationsCtx } from '@/lib/conversations-context'
import { getIiiClient } from '@/lib/iii-client'
import type { CommEvent } from '@/types/iii-agent-event'
import { TimelineGrid } from './TimelineGrid'

export function Timeline() {
  const [sessionId] = useTimelineRoute()
  const { conversations, select } = useConversationsCtx()
  const [events, setEvents] = useState<CommEvent[]>([])
  const [liveOnly, setLiveOnly] = useState(false)
  const [truncated, setTruncated] = useState(false)

  const rootId = useMemo(() => {
    if (!sessionId) return null
    return resolveRootId(
      sessionId,
      (id) => conversations.find((c) => c.id === id)?.parentId ?? null,
    )
  }, [sessionId, conversations])

  useEffect(() => {
    if (!rootId) return
    setEvents([])
    setLiveOnly(false)
    setTruncated(false)
    let disposed = false
    let offLive: (() => void) | undefined
    ;(async () => {
      const client = await getIiiClient()
      if (disposed) return
      offLive = startCommEventsSubscription(client, rootId, (e) => {
        setEvents((prev) => mergeEvents(prev, [e]))
      })
      try {
        const history = await fetchCommHistory(rootId)
        if (disposed) return
        setEvents((prev) => mergeEvents(history.events, prev))
        setTruncated(history.truncated)
      } catch {
        if (!disposed) setLiveOnly(true)
      }
    })()
    return () => {
      disposed = true
      offLive?.()
    }
  }, [rootId])

  if (!rootId) {
    return (
      <div className="flex-1 flex items-center justify-center font-mono text-[13px] text-ink-faint lowercase">
        select a conversation, then open its timeline — or use a
        #/timeline/&lt;session-id&gt; link.
      </div>
    )
  }

  const lanes = buildLanes(rootId, events)
  const laneTitle = (id: string) =>
    conversations.find((c) => c.id === id)?.title ?? id

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-y-auto">
      {liveOnly ? (
        <div className="px-3 py-2 font-mono text-[11px] text-warn border-b border-rule">
          history unavailable — showing live events only
        </div>
      ) : null}
      {truncated ? (
        <div className="px-3 py-2 font-mono text-[11px] text-ink-faint border-b border-rule">
          showing the last 500 events
        </div>
      ) : null}
      <TimelineGrid
        rootId={rootId}
        lanes={lanes}
        events={events}
        laneTitle={laneTitle}
        onOpenSession={(id) => {
          select(id)
        }}
      />
    </div>
  )
}

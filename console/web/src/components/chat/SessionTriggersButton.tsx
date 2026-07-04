import { Zap } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import {
  type RegisteredTriggerSummary,
  registeredTriggersListResponseSchema,
  safeParseResponse,
} from '@/components/chat/engine/parsers'
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/Dialog'
import { fetchCommHistory } from '@/lib/backend/comm-events-live'
import { resolveRootId } from '@/lib/comm/timeline'
import { useConversationsCtx } from '@/lib/conversations-context'
import { getIiiClient } from '@/lib/iii-client'
import type { CommEvent } from '@/types/iii-agent-event'

interface SessionTriggersButtonProps {
  sessionId: string
}

/**
 * Header button opening the per-session triggers panel: which triggers this
 * session registered, and when triggers fired in its family. The dock has no
 * tab chrome, so this ships as a Dialog — same placement pattern as
 * ExportSessionButton.
 */
export function SessionTriggersButton({
  sessionId,
}: SessionTriggersButtonProps) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label="session triggers"
        title="triggers registered by this session"
        className="flex items-center justify-center size-6 text-ink-faint hover:text-ink transition-colors focus-visible:outline-none focus-visible:text-accent"
      >
        <Zap className="size-3.5" />
      </button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogTitle className="text-[11px] uppercase tracking-[0.18em] text-ink-faint">
            session triggers
          </DialogTitle>
          {open ? <SessionTriggersBody sessionId={sessionId} /> : null}
        </DialogContent>
      </Dialog>
    </>
  )
}

function SessionTriggersBody({ sessionId }: { sessionId: string }) {
  const { conversations } = useConversationsCtx()
  const [related, setRelated] = useState<RegisteredTriggerSummary[] | null>(
    null,
  )
  const [fires, setFires] = useState<CommEvent[] | null>(null)

  const rootId = useMemo(() => {
    // Harness-stamped root first; parentId walk is the pre-stamp fallback.
    const stamped = conversations.find((c) => c.id === sessionId)?.rootId
    if (stamped) return stamped
    return resolveRootId(
      sessionId,
      (id) => conversations.find((c) => c.id === id)?.parentId ?? null,
    )
  }, [sessionId, conversations])

  useEffect(() => {
    let disposed = false
    ;(async () => {
      try {
        const client = await getIiiClient()
        const raw = await client.trigger(
          'engine::registered-triggers::list',
          {},
        )
        const resp = safeParseResponse(
          registeredTriggersListResponseSchema,
          raw,
        )
        if (!disposed && resp) {
          // Real ownership lives in registration metadata the list endpoint
          // does not expose (the harness stamps __owner_session_id there), so
          // this is a REFERENCE match: triggers whose config mentions this
          // session. Console-internal browser handlers (iii::*) are excluded.
          // ponytail: swap to an owner-stamp filter when
          // engine::registered-triggers::list exposes metadata.
          setRelated(
            resp.registered_triggers.filter(
              (t) =>
                !t.function_id.startsWith('iii::') &&
                `${t.config_summary} ${t.id}`.includes(sessionId),
            ),
          )
        }
      } catch {
        if (!disposed) setRelated([])
      }
      try {
        const history = await fetchCommHistory(rootId)
        if (!disposed) {
          setFires(history.events.filter((e) => e.kind === 'trigger_fire'))
        }
      } catch {
        if (!disposed) setFires([])
      }
    })()
    return () => {
      disposed = true
    }
  }, [sessionId, rootId])

  return (
    <div className="mt-3 flex flex-col gap-4 font-mono text-[12px]">
      <section>
        <div className="text-[10px] uppercase tracking-[0.06em] text-ink-ghost mb-1">
          triggers referencing this session
        </div>
        {related === null ? (
          <div className="text-ink-ghost animate-pulse">· loading…</div>
        ) : related.length === 0 ? (
          <div className="text-ink-ghost">· none found</div>
        ) : (
          <ul className="divide-y divide-rule-2 border-t border-b border-rule-2">
            {related.map((t) => (
              <li
                key={t.id}
                className="py-1.5 flex items-baseline gap-2 flex-wrap"
              >
                <span className="text-[11px] uppercase tracking-[0.06em] text-ink-faint">
                  {t.trigger_type}
                </span>
                <span className="text-ink-faint">→</span>
                <span className="text-accent break-all">{t.function_id}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
      <section>
        <div className="text-[10px] uppercase tracking-[0.06em] text-ink-ghost mb-1">
          fires in this family
        </div>
        {fires === null ? (
          <div className="text-ink-ghost animate-pulse">· loading…</div>
        ) : fires.length === 0 ? (
          <div className="text-ink-ghost">· no fires recorded</div>
        ) : (
          <ul className="divide-y divide-rule-2 border-t border-b border-rule-2">
            {fires.map((e) => (
              <li
                key={`${e.seq}-${e.at}`}
                className="py-1.5 flex items-baseline gap-2"
              >
                <span className="text-ink-ghost tabular-nums shrink-0">
                  {new Date(e.at).toLocaleTimeString(undefined, {
                    hour12: false,
                  })}
                </span>
                <span className="text-warn shrink-0" aria-hidden>
                  ⚡
                </span>
                <span className="text-ink-faint truncate">
                  {e.trigger?.label ?? e.trigger?.registered_trigger_id ?? 'trigger'}
                  {e.trigger?.action ? ` · ${e.trigger.action}` : ''}
                </span>
              </li>
            ))}
          </ul>
        )}
        <a
          href={`#/timeline/${encodeURIComponent(sessionId)}`}
          className="inline-block mt-2 text-[11px] text-ink-faint hover:text-accent lowercase"
        >
          open full timeline →
        </a>
      </section>
    </div>
  )
}

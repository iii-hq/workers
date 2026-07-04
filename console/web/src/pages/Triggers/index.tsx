import { RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  type RegisteredTriggerSummary,
  registeredTriggersListResponseSchema,
  safeParseResponse,
} from '@/components/chat/engine/parsers'
import { getIiiClient } from '@/lib/iii-client'

export function Triggers() {
  const [triggers, setTriggers] = useState<RegisteredTriggerSummary[] | null>(
    null,
  )
  const [error, setError] = useState<string | null>(null)
  const [search, setSearch] = useState('')

  const load = useCallback(async () => {
    setError(null)
    try {
      const client = await getIiiClient()
      const raw = await client.trigger('engine::registered-triggers::list', {})
      const resp = safeParseResponse(registeredTriggersListResponseSchema, raw)
      if (!resp) throw new Error('unexpected response shape')
      setTriggers(resp.registered_triggers)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  const filtered = useMemo(() => {
    if (!triggers) return null
    const q = search.trim().toLowerCase()
    if (!q) return triggers
    return triggers.filter((t) =>
      [t.id, t.worker_name, t.trigger_type, t.function_id, t.config_summary]
        .join(' ')
        .toLowerCase()
        .includes(q),
    )
  }, [triggers, search])

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-y-auto">
      <div className="flex items-center gap-3 px-3 py-2 border-b border-rule">
        <input
          value={search}
          onChange={(e) => setSearch(e.currentTarget.value)}
          placeholder="filter triggers…"
          className="flex-1 bg-transparent border border-rule px-2 py-1 font-mono text-[12px] text-ink placeholder:text-ink-ghost outline-none focus-visible:border-accent lowercase"
        />
        <button
          type="button"
          onClick={() => void load()}
          aria-label="refresh registered triggers"
          title="refresh"
          className="flex items-center justify-center size-7 border border-rule text-ink-faint hover:text-ink hover:border-ink transition-colors"
        >
          <RefreshCw className="size-3.5" />
        </button>
      </div>
      {error ? (
        <div className="px-3 py-3 font-mono text-[12px] text-alert">
          failed to list triggers: {error}
        </div>
      ) : filtered === null ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · listing registered triggers…
        </div>
      ) : (
        <TriggersList triggers={filtered} />
      )}
    </div>
  )
}

export function TriggersList({
  triggers,
}: {
  triggers: RegisteredTriggerSummary[]
}) {
  if (triggers.length === 0) {
    return (
      <div className="px-3 py-4 font-mono text-[12.5px] text-ink-ghost">
        · no registered triggers
      </div>
    )
  }
  return (
    <ul className="divide-y divide-rule-2">
      {triggers.map((t) => (
        <li key={t.id} className="px-3 py-2 flex flex-col gap-1">
          <div className="flex items-baseline gap-2 flex-wrap">
            <span
              className="font-mono text-[11px] text-ink-faint break-all"
              title={t.id}
            >
              {t.id}
            </span>
            <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint border border-rule-2 bg-paper-2 px-1.5 py-0.5">
              {t.worker_name}
            </span>
          </div>
          <div className="flex items-baseline gap-2 flex-wrap">
            <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
              {t.trigger_type}
            </span>
            <span className="font-mono text-[11px] text-ink-faint">→</span>
            <span className="font-mono text-[12.5px] text-accent break-all">
              {t.function_id}
            </span>
          </div>
          {t.config_summary ? (
            <div className="font-mono text-[11.5px] text-ink-faint leading-[1.5] break-all">
              {t.config_summary}
            </div>
          ) : null}
        </li>
      ))}
    </ul>
  )
}

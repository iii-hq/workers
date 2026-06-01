import type { ReactNode } from 'react'
import { CodeHighlight } from '@/lib/syntax'
import {
  type RegisteredTriggersListRequest,
  registeredTriggersListRequestSchema,
  registeredTriggersListResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'
import { FilterChip, InternalChip, ListHeader } from './shared'

interface RegisteredTriggersListViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function RegisteredTriggersListView({
  input,
  output,
  running,
}: RegisteredTriggersListViewProps) {
  const req = safeParseRequest(registeredTriggersListRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <ListHeader
          count={0}
          noun="registered triggers"
          tone="default"
          filters={<RequestFilters req={req ?? undefined} />}
        />
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · listing registered triggers…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(registeredTriggersListResponseSchema, output)
  if (!resp) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <ListHeader
        count={resp.registered_triggers.length}
        noun="registered triggers"
        filters={<RequestFilters req={req ?? undefined} />}
      />
      {resp.registered_triggers.length === 0 ? (
        <div className="px-3 py-4 font-mono text-[12.5px] text-ink-ghost">
          · no registered triggers returned
        </div>
      ) : (
        <ul className="divide-y divide-rule-2">
          {resp.registered_triggers.map((t) => (
            <li key={t.id} className="px-3 py-2 flex flex-col gap-1">
              <div className="flex items-baseline gap-2 flex-wrap">
                <span
                  className="font-mono text-[11px] text-ink-faint break-all"
                  title={t.id}
                >
                  {shortenId(t.id)}
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
              <ConfigSummary text={t.config_summary} />
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function shortenId(id: string): string {
  if (id.length <= 14) return id
  return `${id.slice(0, 8)}…${id.slice(-4)}`
}

function ConfigSummary({ text }: { text: string }) {
  if (!text) return null
  const trimmed = text.trim()
  if (!trimmed) return null
  if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
    return (
      <CodeHighlight
        code={prettyJsonIfPossible(trimmed)}
        language="json"
        wrap
        className="border border-rule-2 text-[11.5px] leading-[1.5] px-2 py-1.5"
      />
    )
  }
  return (
    <div className="font-mono text-[11.5px] text-ink-faint leading-[1.5]">
      {trimmed}
    </div>
  )
}

function prettyJsonIfPossible(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2)
  } catch {
    return text
  }
}

function RequestFilters({ req }: { req?: RegisteredTriggersListRequest }) {
  if (!req) return null
  const chips: ReactNode[] = []
  if (req.function_id) {
    chips.push(
      <FilterChip
        key="function_id"
        label="function_id"
        value={req.function_id}
      />,
    )
  }
  if (req.trigger_type) {
    chips.push(
      <FilterChip
        key="trigger_type"
        label="trigger_type"
        value={req.trigger_type}
      />,
    )
  }
  if (req.worker) {
    chips.push(<FilterChip key="worker" label="worker" value={req.worker} />)
  }
  if (req.search) {
    chips.push(<FilterChip key="search" label="search" value={req.search} />)
  }
  if (req.include_internal) {
    chips.push(<InternalChip key="internal" />)
  }
  return chips.length > 0 ? chips : null
}

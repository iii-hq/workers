import type { ReactNode } from 'react'
import {
  safeParseRequest,
  safeParseResponse,
  type TriggersListRequest,
  triggersListRequestSchema,
  triggersListResponseSchema,
} from './parsers'
import { FilterChip, InternalChip, ListHeader } from './shared'

interface TriggersListViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function TriggersListView({
  input,
  output,
  running,
}: TriggersListViewProps) {
  const req = safeParseRequest(triggersListRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <ListHeader
          count={0}
          noun="triggers"
          tone="default"
          filters={<RequestFilters req={req ?? undefined} />}
        />
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · listing triggers…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(triggersListResponseSchema, output)
  if (!resp) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <ListHeader
        count={resp.triggers.length}
        noun="triggers"
        filters={<RequestFilters req={req ?? undefined} />}
      />
      {resp.triggers.length === 0 ? (
        <div className="px-3 py-4 font-mono text-[12.5px] text-ink-ghost">
          · no triggers returned
        </div>
      ) : (
        <ul className="divide-y divide-rule-2">
          {resp.triggers.map((t) => (
            <li
              key={`${t.worker_name}:${t.id}`}
              className="px-3 py-2 flex flex-col gap-0.5"
            >
              <div className="flex items-baseline gap-2 flex-wrap">
                <span className="font-mono text-[12.5px] text-accent break-all">
                  {t.id}
                </span>
                <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint border border-rule-2 bg-paper-2 px-1.5 py-0.5">
                  {t.worker_name}
                </span>
              </div>
              <div className="font-mono text-[12px] text-ink-faint leading-[1.55]">
                {t.description}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function RequestFilters({ req }: { req?: TriggersListRequest }) {
  if (!req) return null
  const chips: ReactNode[] = []
  if (req.prefix) {
    chips.push(<FilterChip key="prefix" label="prefix" value={req.prefix} />)
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

import type { ReactNode } from 'react'
import {
  type FunctionsListRequest,
  functionsListRequestSchema,
  functionsListResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'
import { FilterChip, InternalChip, ListHeader } from './shared'

interface FunctionsListViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function FunctionsListView({
  input,
  output,
  running,
}: FunctionsListViewProps) {
  const req = safeParseRequest(functionsListRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <ListHeader
          count={0}
          noun="functions"
          tone="default"
          filters={<RequestFilters req={req ?? undefined} />}
        />
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · listing functions…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(functionsListResponseSchema, output)
  if (!resp) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <ListHeader
        count={resp.functions.length}
        noun="functions"
        filters={<RequestFilters req={req ?? undefined} />}
      />
      {resp.functions.length === 0 ? (
        <div className="px-3 py-4 font-mono text-[12.5px] text-ink-ghost">
          · no functions returned
        </div>
      ) : (
        <ul className="divide-y divide-rule-2">
          {resp.functions.map((fn) => (
            <li
              key={`${fn.worker_name}:${fn.function_id}`}
              className="px-3 py-2 flex flex-col gap-0.5"
            >
              <div className="flex items-baseline gap-2 flex-wrap">
                <span className="font-mono text-[12.5px] text-accent break-all">
                  {fn.function_id}
                </span>
                <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint border border-rule-2 bg-paper-2 px-1.5 py-0.5">
                  {fn.worker_name}
                </span>
              </div>
              {fn.description ? (
                <div className="font-mono text-[12px] text-ink-faint leading-[1.55]">
                  {fn.description}
                </div>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function RequestFilters({ req }: { req?: FunctionsListRequest }) {
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

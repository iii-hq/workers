import type { ReactNode } from 'react'
import { filterChips } from '@/components/observer-activity/ObserverSource'
import {
  type ObserverInfo,
  type ObserversListRequest,
  observersListRequestSchema,
  observersListResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'
import { FilterChip, ListHeader } from './shared'

interface ObserversListViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/**
 * `engine::observers::list` — every observer that matched the request: the
 * observed function, its args and filters, the target it runs, its state
 * and how often it fired.
 */
export function ObserversListView({
  input,
  output,
  running,
}: ObserversListViewProps) {
  const req = safeParseRequest(observersListRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <ListHeader
          count={0}
          noun="observers"
          tone="default"
          filters={<RequestFilters req={req ?? undefined} />}
        />
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · listing observers…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(observersListResponseSchema, output)
  if (!resp) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <ListHeader
        count={resp.observers.length}
        noun="observers"
        filters={<RequestFilters req={req ?? undefined} />}
      />
      {resp.observers.length === 0 ? (
        <div className="px-3 py-4 font-mono text-[12.5px] text-ink-ghost">
          · no observers returned
        </div>
      ) : (
        <ul className="divide-y divide-rule-2">
          {resp.observers.map((observer) => (
            <ObserverRow key={observer.id} observer={observer} />
          ))}
        </ul>
      )}
    </div>
  )
}

export function ObserverRow({ observer }: { observer: ObserverInfo }) {
  const args = argChips(observer.args)
  const filters = filterChips(observer.filters)
  return (
    <li className="px-3 py-2 flex flex-col gap-1">
      <div className="flex items-baseline gap-2 flex-wrap">
        <span
          className="font-mono text-[11px] text-ink-faint break-all"
          title={observer.id}
        >
          {shortenId(observer.id)}
        </span>
        <span className="iii-ui-eyebrow border border-rule-2 bg-paper-2 px-1.5 py-0.5">
          {observer.state}
        </span>
        {observer.worker?.worker_name ? (
          <span className="iii-ui-eyebrow border border-rule-2 bg-paper-2 px-1.5 py-0.5">
            {observer.worker.worker_name}
          </span>
        ) : null}
        {typeof observer.fires === 'number' ? (
          <span className="font-mono text-[11px] text-ink-ghost tabular-nums">
            {observer.fires} {observer.fires === 1 ? 'fire' : 'fires'}
          </span>
        ) : null}
      </div>
      <div className="flex items-baseline gap-2 flex-wrap">
        <span className="font-mono text-[12.5px] text-ink break-all">
          {observer.function_id}
        </span>
        <span className="font-mono text-[11px] text-ink-faint">→</span>
        <span className="font-mono text-[12.5px] text-accent break-all">
          {observer.target.function_id}
        </span>
      </div>
      {args.length > 0 || filters.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          {args.map(({ field, value }) => (
            <FilterChip key={`args.${field}`} label={field} value={value} />
          ))}
          {filters.map(({ field, value }) => (
            <FilterChip key={field} label={field} value={value} />
          ))}
        </div>
      ) : null}
      {observer.reason ? (
        <div className="font-mono text-[11.5px] text-warn leading-[1.5]">
          {observer.reason.code}: {observer.reason.message}
        </div>
      ) : null}
    </li>
  )
}

/** Top-level args as `field value` chips; nested values as compact JSON. */
function argChips(args: unknown): { field: string; value: string }[] {
  if (!args || typeof args !== 'object' || Array.isArray(args)) return []
  return Object.entries(args as Record<string, unknown>).map(
    ([field, value]) => ({
      field,
      value:
        value === null || typeof value !== 'object'
          ? String(value)
          : (JSON.stringify(value) ?? ''),
    }),
  )
}

function shortenId(id: string): string {
  if (id.length <= 14) return id
  return `${id.slice(0, 8)}…${id.slice(-4)}`
}

function RequestFilters({ req }: { req?: ObserversListRequest }) {
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
  if (req.namespace) {
    chips.push(
      <FilterChip key="namespace" label="namespace" value={req.namespace} />,
    )
  }
  if (req.worker) {
    chips.push(<FilterChip key="worker" label="worker" value={req.worker} />)
  }
  return chips.length > 0 ? chips : null
}

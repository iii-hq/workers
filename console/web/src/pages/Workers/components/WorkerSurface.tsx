/**
 * What one worker brought to the bus: the functions it registered, the
 * trigger types it publishes, and the live bindings pointing into it.
 *
 * Rendered inline under an expanded row on the Workers table, so "what can
 * this worker actually do" is answered where the operator already is instead
 * of on another page. One `engine::workers::info` call carries all three
 * lists, and it only runs when a row is actually opened.
 *
 * Deeper reads stay on the dedicated pages: a function's schemas, invoke
 * panel and call history live on `#/ext/functions`, a binding's fire path on
 * `#/ext/triggers`. This is the index, not the manual.
 */

import { useQuery } from '@tanstack/react-query'
import { AlertCircle, ChevronRight } from 'lucide-react'
import { Badge } from '@/components/ui/Badge'
import { Skeleton } from '@/components/ui/Skeleton'
import { fetchEngineWorkerInfo } from '../api/workers'

interface WorkerSurfaceProps {
  name: string
}

export const workerSurfaceKeys = {
  detail: (name: string) => ['workers', 'surface', name] as const,
}

export function WorkerSurface({ name }: WorkerSurfaceProps) {
  const query = useQuery({
    queryKey: workerSurfaceKeys.detail(name),
    queryFn: () => fetchEngineWorkerInfo(name),
  })

  if (query.isLoading) {
    return (
      <div className="space-y-2 py-3">
        <Skeleton className="h-4 w-48" />
        <Skeleton className="h-4 w-72" />
        <Skeleton className="h-4 w-64" />
      </div>
    )
  }

  if (query.isError || !query.data) {
    return (
      <div className="flex items-center gap-2 py-3 font-mono text-[12px] text-alert">
        <AlertCircle className="h-3.5 w-3.5 shrink-0" aria-hidden />
        engine::workers::info returned nothing for {name} — the worker may have
        disconnected.
      </div>
    )
  }

  const { functions, trigger_types: triggerTypes } = query.data
  const bindings = query.data.registered_triggers

  return (
    <div className="grid gap-x-8 gap-y-6 py-3 md:grid-cols-2 xl:grid-cols-3">
      <Section title="functions" count={functions.length}>
        {functions.length === 0 ? (
          <Empty>this worker registered no functions.</Empty>
        ) : (
          functions.map((fn) => (
            <div key={fn.function_id} className="space-y-0.5">
              <div className="font-mono text-[12.5px] text-ink break-words">
                {fn.function_id}
              </div>
              {fn.description ? (
                <p className="font-mono text-[11.5px] leading-relaxed text-ink-faint">
                  {fn.description}
                </p>
              ) : null}
            </div>
          ))
        )}
      </Section>

      <Section title="trigger types" count={triggerTypes.length}>
        {triggerTypes.length === 0 ? (
          <Empty>this worker publishes no trigger types.</Empty>
        ) : (
          triggerTypes.map((type) => (
            <div key={type.id} className="space-y-0.5">
              <div className="font-mono text-[12.5px] text-ink break-words">
                {type.id}
              </div>
              {type.description ? (
                <p className="font-mono text-[11.5px] leading-relaxed text-ink-faint">
                  {type.description}
                </p>
              ) : null}
            </div>
          ))
        )}
      </Section>

      <Section title="bindings" count={bindings.length}>
        {bindings.length === 0 ? (
          <Empty>nothing is bound to this worker's functions.</Empty>
        ) : (
          bindings.map((binding) => (
            // Stacked, not side by side: a trigger type and a function id are
            // both long, and sharing a line squeezes both into mid-word breaks
            // in a column this narrow.
            <div key={binding.id} className="space-y-1">
              <div>
                <Badge>{binding.trigger_type}</Badge>
              </div>
              <div className="font-mono text-[12px] text-ink break-words">
                {binding.function_id}
              </div>
              {binding.config_summary && binding.config_summary !== '{}' ? (
                <p className="font-mono text-[11.5px] text-ink-faint break-words">
                  {binding.config_summary}
                </p>
              ) : null}
            </div>
          ))
        )}
      </Section>
    </div>
  )
}

function Section({
  title,
  count,
  children,
}: {
  title: string
  count: number
  children: React.ReactNode
}) {
  return (
    <section className="min-w-0 space-y-2">
      <h3 className="flex items-center gap-2 font-mono text-[11px] font-medium uppercase tracking-[0.1em] text-ink-faint">
        <ChevronRight className="h-3 w-3" aria-hidden />
        {title}
        <span className="tabular-nums text-ink-ghost">{count}</span>
      </h3>
      <div className="space-y-2">{children}</div>
    </section>
  )
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <p className="font-mono text-[11.5px] leading-relaxed text-ink-ghost">
      {children}
    </p>
  )
}

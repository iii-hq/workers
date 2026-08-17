/**
 * What one worker brought to the bus: the functions it registered and the
 * trigger types it publishes.
 *
 * Rendered inline under an expanded row on the Workers table, so "what can
 * this worker actually do" is answered where the operator already is instead
 * of on another page. One `engine::workers::info` call carries both lists,
 * and it only runs when a row is actually opened.
 *
 * Registered triggers deliberately do NOT appear here. They are live
 * bindings of a type to a function, they belong to the triggers view, and
 * showing them per worker invited the question of whose they are — the
 * registering worker's or the target's. Deeper reads stay on the dedicated
 * pages: schemas, invoke and call history on `#/ext/functions`, registered
 * triggers and their fire paths on `#/ext/triggers`.
 */

import { useQuery } from '@tanstack/react-query'
import { AlertCircle, ChevronRight } from 'lucide-react'
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

  return (
    <div className="grid gap-x-8 gap-y-6 py-3 md:grid-cols-2">
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

/**
 * What one worker brought to the bus: the functions it registered, marked
 * when they are private or take observer args, and how many observers it
 * created.
 *
 * Rendered inline under an expanded row on the Workers table, so "what can
 * this worker actually do" is answered where the operator already is instead
 * of on another page. One `engine::workers::info` call carries both, and it
 * only runs when a row is actually opened.
 *
 * The observers themselves stay on the `observers` page: each one ties an
 * observed function to a target, and listing them per worker invited the
 * question of whose they are — the observing worker's or the observed
 * function's. Deeper reads stay on the dedicated pages: schemas, invoke and
 * call history on the `functions` page, observers and their fire paths on
 * the `observers` page.
 */

import { useQuery } from '@tanstack/react-query'
import { AlertCircle, ChevronRight } from 'lucide-react'
import { Eyebrow } from '@/components/ui/Eyebrow'
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
        <AlertCircle className="size-4 shrink-0" aria-hidden />
        engine::workers::info returned nothing for {name} — the worker may have
        disconnected.
      </div>
    )
  }

  const { functions } = query.data
  const observers = query.data.observers ?? []

  return (
    <div className="grid gap-x-8 gap-y-6 py-3 @3xl:grid-cols-2">
      <Section title="functions" count={functions.length}>
        {functions.length === 0 ? (
          <Empty>This worker registered no functions.</Empty>
        ) : (
          functions.map((fn) => (
            <div key={fn.function_id} className="space-y-0.5">
              <div className="flex flex-wrap items-baseline gap-x-2 font-mono text-[12.5px] text-ink break-words">
                {fn.function_id}
                {facts(fn).map((fact) => (
                  <span key={fact} className="text-[11px] text-ink-ghost">
                    {fact}
                  </span>
                ))}
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

      <Section title="observers created" count={observers.length}>
        {observers.length === 0 ? (
          <Empty>This worker observes no functions.</Empty>
        ) : (
          observers.map((id) => (
            <div
              key={id}
              className="font-mono text-[12.5px] text-ink-faint break-all"
            >
              {id}
            </div>
          ))
        )}
      </Section>
    </div>
  )
}

/** The short facts a function row carries: private, takes args, observed. */
function facts(fn: {
  private?: boolean
  args_format?: unknown
  observable?: boolean
  observer_count?: number
}): string[] {
  const out: string[] = []
  if (fn.private) out.push('private')
  if (fn.args_format !== undefined && fn.args_format !== null)
    out.push('takes args')
  if (fn.observable === false) out.push('not observable')
  if (fn.observer_count)
    out.push(
      `${fn.observer_count} ${fn.observer_count === 1 ? 'observer' : 'observers'}`,
    )
  return out
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
      <Eyebrow as="h3" className="flex items-center gap-2">
        <ChevronRight className="size-4" aria-hidden />
        {title}
        <span className="tabular-nums text-ink-ghost">{count}</span>
      </Eyebrow>
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

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
import { AlertCircle, FunctionSquare, Zap } from 'lucide-react'
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
      <div className="flex flex-col gap-2 py-3">
        <Skeleton className="h-10 w-full rounded-lg" />
        <Skeleton className="h-10 w-full rounded-lg" />
        <Skeleton className="h-10 w-2/3 rounded-lg" />
      </div>
    )
  }

  if (query.isError || !query.data) {
    return (
      <div className="flex items-start gap-3 rounded-xl border border-alert/25 bg-alert-muted px-4 py-3.5 font-inter text-[13px] leading-5 text-alert-strong">
        <AlertCircle className="mt-0.5 size-4 shrink-0" aria-hidden />
        <span>
          <span className="wk-machine">engine::workers::info</span> returned
          nothing for {name}. The worker may have disconnected.
        </span>
      </div>
    )
  }

  const { functions } = query.data
  const observers = query.data.observers ?? []

  return (
    <div className="grid gap-4 py-1 @3xl:grid-cols-2">
      <Section
        title="Functions"
        icon={<FunctionSquare className="size-4" aria-hidden />}
        count={functions.length}
        empty="This worker registered no functions."
      >
        {functions.map((fn) => (
          <Row
            key={fn.function_id}
            id={fn.function_id}
            facts={facts(fn)}
            description={fn.description}
          />
        ))}
      </Section>
      <Section
        title="Observers created"
        icon={<Zap className="size-4" aria-hidden />}
        count={observers.length}
        empty="This worker observes no functions."
      >
        {observers.map((id) => (
          <Row key={id} id={id} />
        ))}
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
  icon,
  count,
  empty,
  children,
}: {
  title: string
  icon: React.ReactNode
  count: number
  empty: string
  children: React.ReactNode
}) {
  return (
    <section className="wk-card min-w-0 overflow-hidden rounded-xl border border-neutral-200 font-inter dark:border-neutral-800">
      <h3 className="flex items-center gap-2 border-b border-neutral-200 px-4 py-3 text-[13px] font-medium leading-5 text-neutral-900 dark:border-neutral-800 dark:text-neutral-50">
        <span className="text-neutral-500 dark:text-neutral-400">{icon}</span>
        {title}
        <span className="ml-auto rounded-lg border border-neutral-200 px-2 py-0.5 text-[12px] font-medium tabular-nums text-neutral-500 dark:border-neutral-800 dark:text-neutral-400">
          {count}
        </span>
      </h3>
      {count === 0 ? (
        <p className="px-4 py-4 text-[13px] leading-5 text-neutral-500 dark:text-neutral-400">
          {empty}
        </p>
      ) : (
        <div className="flex flex-col divide-y divide-neutral-200 dark:divide-neutral-800">
          {children}
        </div>
      )}
    </section>
  )
}

function Row({
  id,
  facts = [],
  description,
}: {
  id: string
  facts?: readonly string[]
  description?: string | null
}) {
  return (
    <div className="flex flex-col gap-1 px-4 py-3">
      <div className="flex flex-wrap items-baseline gap-x-2">
        <span className="wk-machine break-words text-neutral-900 dark:text-neutral-100">
          {id}
        </span>
        {facts.map((fact) => (
          <span
            key={fact}
            className="text-[12px] text-neutral-500 dark:text-neutral-400"
          >
            {fact}
          </span>
        ))}
      </div>
      {description ? (
        <p className="text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
          {description}
        </p>
      ) : null}
    </div>
  )
}

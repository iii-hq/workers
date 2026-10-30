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
 * pages: schemas, invoke and call history on the `functions` page, registered
 * triggers and their fire paths on the `triggers` page.
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

  const { functions, trigger_types: triggerTypes } = query.data

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
            description={fn.description}
          />
        ))}
      </Section>
      <Section
        title="Trigger types"
        icon={<Zap className="size-4" aria-hidden />}
        count={triggerTypes.length}
        empty="This worker publishes no trigger types."
      >
        {triggerTypes.map((type) => (
          <Row key={type.id} id={type.id} description={type.description} />
        ))}
      </Section>
    </div>
  )
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

function Row({ id, description }: { id: string; description?: string | null }) {
  return (
    <div className="flex flex-col gap-1 px-4 py-3">
      <div className="wk-machine break-words text-neutral-900 dark:text-neutral-100">
        {id}
      </div>
      {description ? (
        <p className="text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
          {description}
        </p>
      ) : null}
    </div>
  )
}

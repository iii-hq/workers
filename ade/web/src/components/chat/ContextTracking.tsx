import { formatRelative } from '@iii-dev/console-ui/format'
import { Check, Circle, ExternalLink } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { getIiiClient, type IIIConnectionState } from '@/lib/iii-client'

export interface ContextTrackingItem {
  id: string
  scope: 'conversation' | 'project'
  owner: string
  title: string
  summary: string
  progress: number | null
  items: { label: string; done: boolean }[]
  source: { label: string; url: string | null } | null
  updated_at: number
}

export function safeTrackingUrl(raw: string | null | undefined): string | null {
  if (!raw) return null
  try {
    const url = new URL(raw)
    return ['https:', 'http:'].includes(url.protocol) &&
      !url.username &&
      !url.password
      ? url.href
      : null
  } catch {
    return null
  }
}

/** Reload snapshots, never their domain sources. No polling or background agent. */
export function useContextTracking({
  sessionId,
  workingDir,
  status,
  active,
  connectionState,
}: {
  sessionId: string
  workingDir: string | null
  status?: string
  active: boolean
  connectionState: IIIConnectionState
}) {
  const [tick, setTick] = useState(0)
  const [state, setState] = useState<{
    key: string
    items: ContextTrackingItem[]
    loading: boolean
    error: boolean
  } | null>(null)
  const key = JSON.stringify([sessionId, workingDir])
  useEffect(() => {
    const focus = () => setTick((n) => n + 1)
    window.addEventListener('focus', focus)
    return () => window.removeEventListener('focus', focus)
  }, [])
  // biome-ignore lint/correctness/useExhaustiveDependencies: status, connectionState and tick invalidate the saved snapshots
  useEffect(() => {
    if (!active) return
    let alive = true
    setState((previous) => ({
      key,
      items: previous?.key === key ? previous.items : [],
      loading: true,
      error: false,
    }))
    void getIiiClient()
      .then((client) =>
        client.trigger<{ items: ContextTrackingItem[] }>(
          'console::context::list',
          { session_id: sessionId },
        ),
      )
      .then((result) => {
        if (!Array.isArray(result.items))
          throw new Error('Invalid tracking response')
        if (alive)
          setState({ key, items: result.items, loading: false, error: false })
      })
      .catch(() => {
        if (alive)
          setState((previous) => ({
            key,
            items: previous?.key === key ? previous.items : [],
            loading: false,
            error: true,
          }))
      })
    return () => {
      alive = false
    }
  }, [sessionId, key, active, status, connectionState, tick])
  return {
    items: state?.key === key ? state.items : [],
    loading: state?.key === key ? state.loading : active,
    error: state?.key === key ? state.error : false,
    retry: () => setTick((n) => n + 1),
  }
}

export function ContextTrackingBody({
  items,
  loading,
  error,
  retry,
  onAskAgent,
}: {
  items: ContextTrackingItem[]
  loading: boolean
  error: boolean
  retry: () => void
  onAskAgent?: (text: string, send: boolean) => void
}) {
  if (loading && items.length === 0)
    return (
      <div className="px-3 py-2" role="status" aria-label="Loading tracking">
        <Skeleton className="h-12 w-full" />
      </div>
    )
  return (
    <div className="flex min-w-0 flex-col gap-3 px-3 py-2 font-sans text-[13px] text-ink">
      {error ? (
        <StatusPanel
          variant="warn"
          role="status"
          headline="Tracking could not be loaded"
          detail="Previously loaded snapshots may be out of date."
          action={
            <Button variant="ghost" size="sm" onClick={retry}>
              Retry
            </Button>
          }
        />
      ) : null}
      {!loading && !error && items.length === 0 ? (
        <p className="text-ink-faint">
          Ask in chat to track a release, goal or checklist here.
        </p>
      ) : null}
      {items.map((item) => {
        const url = safeTrackingUrl(item.source?.url)
        const progress =
          typeof item.progress === 'number' &&
          Number.isFinite(item.progress) &&
          item.progress >= 0 &&
          item.progress <= 100
            ? item.progress
            : null
        return (
          <article
            key={item.id}
            aria-label={item.title}
            className="flex min-w-0 flex-col gap-2 [overflow-wrap:anywhere]"
          >
            <div className="flex min-w-0 flex-wrap items-baseline gap-x-2">
              <h3 className="min-w-0 flex-1 font-semibold">{item.title}</h3>
              <span className="text-[11px] text-ink-faint">
                {item.scope === 'project' ? 'Project' : 'Conversation'}
              </span>
              {progress !== null ? (
                <span className="font-mono tabular-nums">{progress}%</span>
              ) : null}
            </div>
            {progress !== null ? (
              <div
                role="progressbar"
                aria-label={`${item.title} progress`}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={progress}
                className="h-1.5 overflow-hidden rounded-md bg-surface"
              >
                <div
                  className="h-full bg-accent"
                  style={{ width: `${progress}%` }}
                />
              </div>
            ) : null}
            <p className="whitespace-pre-wrap text-ink-faint">{item.summary}</p>
            {item.items.length ? (
              <ul
                className="flex flex-col gap-1"
                aria-label={`${item.title} checklist`}
              >
                {item.items.map((entry) => (
                  <li key={entry.label} className="flex items-start gap-2">
                    <span className="sr-only">
                      {entry.done ? 'Completed:' : 'Pending:'}
                    </span>
                    {entry.done ? (
                      <Check
                        aria-hidden
                        className="mt-0.5 size-4 shrink-0 text-ok"
                      />
                    ) : (
                      <Circle
                        aria-hidden
                        className="mt-0.5 size-4 shrink-0 text-ink-faint"
                      />
                    )}
                    <span>{entry.label}</span>
                  </li>
                ))}
              </ul>
            ) : null}
            {item.source ? (
              url ? (
                <a
                  href={url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="flex min-h-11 items-center gap-2 rounded-sm text-ink-faint hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
                >
                  <ExternalLink aria-hidden className="size-4 shrink-0" />
                  {item.source.label}
                </a>
              ) : (
                <p className="text-[11px] text-ink-faint">
                  Source: {item.source.label}
                </p>
              )
            ) : (
              <p className="text-[11px] text-ink-faint">Source not provided</p>
            )}
            <p className="text-[11px] text-ink-faint">
              Snapshot ·{' '}
              <time
                dateTime={new Date(item.updated_at).toISOString()}
                title={new Date(item.updated_at).toLocaleString()}
              >
                Updated {formatRelative(item.updated_at)}
              </time>
            </p>
            {onAskAgent ? (
              <div className="flex flex-wrap gap-1">
                <Button
                  variant="ghost"
                  size="sm"
                  className="min-h-11"
                  onClick={() =>
                    onAskAgent(
                      `Update Context tracking ${JSON.stringify(item.id)} (${JSON.stringify(item.title)}) from its source. Keep the previous snapshot if the source is unavailable.`,
                      false,
                    )
                  }
                >
                  Update in chat
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="min-h-11"
                  onClick={() =>
                    onAskAgent(
                      `Remove Context tracking ${JSON.stringify(item.id)} (${JSON.stringify(item.title)}).`,
                      false,
                    )
                  }
                >
                  Remove in chat
                </Button>
              </div>
            ) : null}
          </article>
        )
      })}
      <p className="text-[11px] text-ink-faint">
        Updated by the agent when it checks the source, not monitored live. Ask
        in chat to edit or change scope.
      </p>
    </div>
  )
}

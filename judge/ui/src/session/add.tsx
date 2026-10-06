import { Button, Skeleton, uiClasses } from '@iii-dev/console-ui'
import { Check, Plus, RefreshCw } from 'lucide-react'
import { useEffect, useState } from 'react'
import { type ConfigureTarget, JudgeMark } from './judges'

/**
 * The public registry's `evaluation` tag, which every judge provider publishes
 * (judge-typesafe has no `judge` tag). One page (20 rows) holds them all.
 */
const REGISTRY_JUDGES_URL = 'https://api.workers.iii.dev/w?tag=evaluation'
const JUDGE_WORKER = /^judge-([a-z0-9-]{1,64})$/

interface RegistryJudge {
  /** Registry slug — also the `compose::add` worker name (`judge-<provider>`). */
  name: string
  provider: string
  version: string | null
  description: string | null
}

/** A judge being added from this session's picker. */
export type AddState =
  | { kind: 'adding'; since: number }
  | { kind: 'done' }
  | { kind: 'failed'; error: string; at: number }

function text(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null
}

/** Every `judge-<provider>` worker in the registry, in its order (downloads). */
async function fetchRegistryJudges(signal: AbortSignal): Promise<RegistryJudge[]> {
  const response = await fetch(REGISTRY_JUDGES_URL, { signal, headers: { accept: 'application/json' } })
  if (!response.ok) throw new Error(`registry returned HTTP ${response.status}`)
  const body = (await response.json()) as { workers?: Record<string, unknown>[] }
  return (body?.workers ?? []).flatMap((row) => {
    const name = text(row?.name)
    const match = name ? JUDGE_WORKER.exec(name) : null
    return name && match
      ? [{ name, provider: match[1], version: text(row.version), description: text(row.description) }]
      : []
  })
}

export interface AddJudgePanelProps {
  /** Providers registered on the engine now (`judge-<provider>` answering). */
  registered: ReadonlySet<string>
  adds: ReadonlyMap<string, AddState>
  /** Settings of registered providers, by provider: an added judge links to them. */
  targets: ReadonlyMap<string, ConfigureTarget>
  onAdd(worker: string): void
  onConfigure(target: ConfigureTarget): void
}

/**
 * The picker's "Add a judge" page, as the model picker's "Add a provider":
 * every judge worker the registry publishes that is not installed yet, one
 * tap to add. Adding runs `compose::add`; the row follows it until the new
 * judge registers, then offers its settings — a hosted judge needs a key.
 */
export function AddJudgePanel({ registered, adds, targets, onAdd, onConfigure }: AddJudgePanelProps) {
  const [registry, setRegistry] = useState<{ judges: RegistryJudge[] | null; error: boolean }>({
    judges: null,
    error: false,
  })
  // Bumped by Retry; the effect reloads.
  const [attempt, setAttempt] = useState(0)
  // biome-ignore lint/correctness/useExhaustiveDependencies: `attempt` is the reload signal
  useEffect(() => {
    const controller = new AbortController()
    setRegistry({ judges: null, error: false })
    fetchRegistryJudges(controller.signal)
      .then((judges) => setRegistry({ judges, error: false }))
      .catch(() => {
        if (!controller.signal.aborted) setRegistry({ judges: [], error: true })
      })
    return () => controller.abort()
  }, [attempt])

  // What is installed stays off this page; a judge added from here stays
  // listed so its progress and outcome remain visible.
  const rows = (registry.judges ?? []).filter((judge) => adds.has(judge.name) || !registered.has(judge.provider))
  const everythingInstalled = (registry.judges?.length ?? 0) > 0 && rows.length === 0

  return (
    <>
      <div className="judge-ui-session-scroll">
        {registry.judges === null ? (
          <div className="judge-ui-session-card" role="status" aria-busy="true" aria-label="Loading judges">
            {[0, 1, 2].map((row) => (
              <div key={row} className="judge-ui-session-add-row">
                <Skeleton className="judge-ui-session-skeleton-mark" />
                <span className="judge-ui-session-add-copy">
                  <Skeleton className="judge-ui-session-skeleton-title" />
                  <Skeleton className="judge-ui-session-skeleton-line" />
                </span>
                <Skeleton className="judge-ui-session-skeleton-action" />
              </div>
            ))}
          </div>
        ) : registry.error ? (
          <div className="judge-ui-session-notice">
            <p>The workers registry is unreachable right now.</p>
            <Button variant="pill" size="sm" onClick={() => setAttempt((n) => n + 1)}>
              Retry
            </Button>
          </div>
        ) : rows.length === 0 ? (
          <p className="judge-ui-session-notice">
            {everythingInstalled
              ? 'Every judge in the registry is already installed.'
              : 'The registry lists no judge workers.'}
          </p>
        ) : (
          <ul className="judge-ui-session-card" aria-label="Judges in the workers registry">
            {rows.map((judge) => {
              const add = adds.get(judge.name)
              const target = add?.kind === 'done' ? targets.get(judge.provider) : undefined
              return (
                <li key={judge.name} className="judge-ui-session-add-row" data-failed={add?.kind === 'failed' || undefined}>
                  <JudgeMark provider={judge.provider} />
                  <span className="judge-ui-session-add-copy">
                    <span className="judge-ui-session-add-title">
                      <span data-label>{judge.provider}</span>
                      <span className="judge-ui-session-worker">
                        {judge.name}
                        {judge.version ? `@${judge.version}` : ''}
                      </span>
                    </span>
                    {judge.description ? (
                      <span className="judge-ui-session-add-description">{judge.description}</span>
                    ) : null}
                    {add?.kind === 'failed' ? (
                      <span className="judge-ui-session-error" role="alert">
                        {add.error}
                      </span>
                    ) : null}
                  </span>
                  {target ? (
                    <Button variant="pill" size="sm" onClick={() => onConfigure(target)}>
                      Configure
                    </Button>
                  ) : add?.kind === 'done' ? (
                    <span className="judge-ui-session-status">
                      <Check size={16} aria-hidden />
                      Added
                    </span>
                  ) : add?.kind === 'adding' ? (
                    <span className="judge-ui-session-status" role="status">
                      <RefreshCw size={16} aria-hidden className={uiClasses.spin} />
                      Adding…
                    </span>
                  ) : (
                    <Button
                      variant="pill"
                      size="sm"
                      aria-label={`${add?.kind === 'failed' ? 'Retry adding' : 'Add'} ${judge.provider}`}
                      onClick={() => onAdd(judge.name)}
                    >
                      {add?.kind === 'failed' ? <RefreshCw size={16} aria-hidden /> : <Plus size={16} aria-hidden />}
                      {add?.kind === 'failed' ? 'Retry' : 'Add'}
                    </Button>
                  )}
                </li>
              )
            })}
          </ul>
        )}
      </div>
      <p className="judge-ui-session-hint">
        Adding a judge runs <code>compose::add</code>, the same as <code>iii trigger compose::add worker=…</code> in your
        terminal. A hosted judge still needs its key: configure it once it is added.
      </p>
    </>
  )
}

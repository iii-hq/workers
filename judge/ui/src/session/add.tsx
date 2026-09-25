import { Button, IconButton, List, ListItem, uiClasses } from '@iii-dev/console-ui'
import { ArrowLeft, Check, Plus, RefreshCw } from 'lucide-react'
import { useEffect, useState } from 'react'

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
  | { kind: 'failed'; error: string }

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
  onAdd(worker: string): void
  onBack(): void
}

/**
 * The picker's second page, as the model picker's "Add a provider": every
 * judge worker the registry publishes that is not installed yet, one tap to
 * add. Adding runs `compose::add`; the row follows it until the new judge
 * registers and shows up on the first page.
 */
export function AddJudgePanel({ registered, adds, onAdd, onBack }: AddJudgePanelProps) {
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
  const rows = (registry.judges ?? []).filter(
    (judge) => adds.has(judge.name) || !registered.has(judge.provider),
  )
  const everythingInstalled = (registry.judges?.length ?? 0) > 0 && rows.length === 0

  return (
    <div className={`judge-ui-session-page ${uiClasses.motionPanel}`} data-page="add">
      <div className="judge-ui-session-subheader">
        <IconButton label="Back to judges" tooltip={false} variant="ghost" onClick={onBack}>
          <ArrowLeft size={16} aria-hidden />
        </IconButton>
        <div className="judge-ui-session-subheader-copy">
          <h2 className="judge-ui-session-subtitle">Add a judge</h2>
          <p className="judge-ui-session-subdescription">Judge workers from the workers registry.</p>
        </div>
      </div>
      {registry.judges === null ? (
        <List className="judge-ui-session-list" aria-busy="true">
          <ListItem className="judge-ui-session-row" disabled label="Loading judges…" />
        </List>
      ) : registry.error ? (
        <div className="judge-ui-session-empty">
          <p>The workers registry is unreachable right now.</p>
          <Button variant="pill" size="sm" onClick={() => setAttempt((n) => n + 1)}>
            Retry
          </Button>
        </div>
      ) : rows.length === 0 ? (
        <p className="judge-ui-session-empty">
          {everythingInstalled
            ? 'Every judge in the registry is already installed.'
            : 'The registry lists no judge workers.'}
        </p>
      ) : (
        <List className="judge-ui-session-list" aria-label="Judges in the workers registry">
          {rows.map((judge) => {
            const add = adds.get(judge.name)
            return (
              <ListItem
                key={judge.name}
                as="div"
                className="judge-ui-session-row judge-ui-session-add-row"
                leading={<span className="judge-ui-session-mark">{judge.provider.charAt(0).toUpperCase()}</span>}
                label={
                  <>
                    {judge.provider}
                    <span className="judge-ui-session-worker">
                      {judge.name}
                      {judge.version ? `@${judge.version}` : ''}
                    </span>
                  </>
                }
                description={
                  add?.kind === 'failed' ? (
                    <span className="judge-ui-session-error" role="alert">
                      {add.error}
                    </span>
                  ) : (
                    (judge.description ?? undefined)
                  )
                }
                trailing={
                  add?.kind === 'done' ? (
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
                  )
                }
              />
            )
          })}
        </List>
      )}
      <p className="judge-ui-session-hint">
        Adding a judge runs <code>compose::add</code>, the same as <code>iii trigger compose::add worker=…</code> in your
        terminal. A hosted judge still needs its credentials: configure it once it appears in the list.
      </p>
    </div>
  )
}

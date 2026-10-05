/**
 * coder::list-templates and coder::scaffold-worker in the chat: the templates
 * a worker can start from, and a new worker from its files to running, in the
 * New worker dialog's own steps. While the card is open, a worker the call
 * started is followed through compose until it runs or fails; a settled
 * outcome is kept, so reopening the card does not follow it again.
 */

import {
  Button,
  Chip,
  EmptyState,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
  StatusPanel,
  uiClasses,
} from '@iii-dev/console-ui'
import { ExternalLink, LayoutPanelLeft, LayoutTemplate, PackagePlus, Plus } from 'lucide-react'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { unwrapEnvelope } from '../lib/envelope'
import { ErrorDisplayView } from '../lib/errors'
import {
  entryFile,
  type FunctionEntry,
  followStart,
  formatElapsed,
  hasAdePage,
  LANGUAGE_LABEL,
  type ListTemplatesResult,
  publicPageHref,
  type ScaffoldResult,
  type StackPhase,
  sourceLabel,
  stackSteps,
  type Trigger,
  workerFunctions,
} from '../page/new-worker'
import { StartFailure, StepList, type StepRow, WorkerFunctions } from '../page/worker-result'
import { parseShellErrorDisplay } from './parsers'
import { FunctionIdLabel } from './shared'

const LIST_ID = 'coder::list-templates'
const SCAFFOLD_ID = 'coder::scaffold-worker'

export const isScaffoldFunction = (functionId: string) => functionId === LIST_ID || functionId === SCAFFOLD_ID

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null
}

const text = (value: unknown) => (typeof value === 'string' && value !== '' ? value : undefined)

function isTemplates(value: unknown): value is ListTemplatesResult {
  const record = asRecord(value)
  return !!record && asRecord(record.source) !== null && Array.isArray(record.templates)
}

function isScaffold(value: unknown): value is ScaffoldResult {
  const record = asRecord(value)
  return (
    !!record && typeof record.name === 'string' && typeof record.directory === 'string' && Array.isArray(record.files)
  )
}

export interface ScaffoldRequest {
  template?: string
  name?: string
  directory?: string
  start?: boolean
}

export function scaffoldRequest(input: unknown): ScaffoldRequest {
  const record = asRecord(unwrapEnvelope(input)) ?? {}
  return {
    template: text(record.template),
    name: text(record.name),
    directory: text(record.directory),
    start: typeof record.start === 'boolean' ? record.start : undefined,
  }
}

const NOT_STARTED = '[harness] Not started'

/** Why the harness let a scaffold write only its files: its note rides in
    the result's content blocks, beside the scaffold's own JSON. */
export function harnessNote(output: unknown): string | null {
  const content = asRecord(output)?.content
  if (!Array.isArray(content)) return null
  for (const block of content) {
    const note = asRecord(block)?.text
    if (typeof note === 'string' && note.startsWith(NOT_STARTED)) return note.slice('[harness] '.length)
  }
  return null
}

/** The human half of the harness note, `Not started (<why>)`: the rest is
    the next call it hands the agent. */
export function notStartedReason(note: string | null): string | null {
  const why = note ? /^Not started \(([^)]*)\)/.exec(note)?.[1] : undefined
  return why ? `${why.charAt(0).toUpperCase()}${why.slice(1)}.` : null
}

/* ── coder::list-templates ──────────────────────────────────────────── */

function TemplatesCard({ host, result }: { host: Host; result: ListTemplatesResult | null }) {
  const count = result?.templates.length ?? 0
  return (
    <section className="shui-scaffold" aria-busy={result === null || undefined}>
      <header className="shui-scaffold-head">
        <LayoutTemplate aria-hidden className="shui-scaffold-icon" />
        <div className="shui-scaffold-heading">
          <span className={result === null ? `shui-scaffold-title ${uiClasses.pulse}` : 'shui-scaffold-title'}>
            {result === null
              ? 'Listing worker templates…'
              : `${count} worker ${count === 1 ? 'template' : 'templates'}`}
          </span>
          {result ? (
            <span className="shui-scaffold-sub" title={sourceLabel(result.source)}>
              {sourceLabel(result.source)}
            </span>
          ) : null}
        </div>
        {result && count > 0 && host.panels ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => host.panels?.open({ pageId: 'ide', context: { type: 'new-worker' } })}
          >
            <Plus aria-hidden />
            New worker
          </Button>
        ) : null}
      </header>
      {result?.source.warning ? (
        <div className="shui-scaffold-body">
          <StatusPanel variant="warn" headline="These templates may be out of date" detail={result.source.warning} />
        </div>
      ) : null}
      {result && count === 0 ? (
        <EmptyState
          title="No worker templates"
          description="Nothing in this source has a worker: block to scaffold from."
        />
      ) : null}
      {result && count > 0 ? (
        <ul className="shui-templates">
          {result.templates.map((template) => (
            <li key={template.id} className="shui-template">
              <div className="shui-template-head">
                <span className="shui-template-name">{template.name}</span>
                <span className="shui-template-id">{template.id}</span>
              </div>
              {template.description ? (
                <p className="shui-template-description" title={template.description}>
                  {template.description}
                </p>
              ) : null}
              <div className="shui-template-tags">
                <Chip>{LANGUAGE_LABEL[template.language] ?? template.language}</Chip>
                {hasAdePage(template.id) ? <Chip>ADE page</Chip> : null}
                {template.requires.map((container) => (
                  <Chip key={container}>needs {container}</Chip>
                ))}
              </div>
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  )
}

/* ── coder::scaffold-worker ─────────────────────────────────────────── */

type Follow =
  | { step: 'adding'; phase: StackPhase }
  | { step: 'running'; phase: StackPhase }
  | { step: 'failed'; phase: StackPhase; error: string; logs: string[] }

/** Settled starts by worker and operation: the card mounts on every open. */
const SETTLED = new Map<string, Follow>()

/** Follows the start a scaffold call asked for, while the card is open, and
    since when: the clock beside the step counts from there, as the dialog's
    does from its Add to stack. */
function useFollowStart(
  host: Host,
  name: string | null,
  operation: string | null,
): { follow: Follow | null; since: number } {
  const key = name === null ? null : `${name} ${operation ?? ''}`
  const [follow, setFollow] = useState<Follow | null>(() =>
    key === null ? null : (SETTLED.get(key) ?? { step: 'adding', phase: 'installing' }),
  )
  const since = useRef(Date.now())
  useEffect(() => {
    if (key === null || name === null || SETTLED.has(key)) return
    since.current = Date.now()
    let closed = false
    let phase: StackPhase = 'installing'
    const trigger: Trigger = <T,>(functionId: string, payload: Record<string, unknown>) =>
      closed ? Promise.reject<T>(new Error('the card closed')) : host.iii.trigger<T>(functionId, payload)
    const sleep = (ms: number) =>
      new Promise<void>((resolve, reject) =>
        setTimeout(() => (closed ? reject(new Error('the card closed')) : resolve()), ms),
      )
    void followStart(
      trigger,
      name,
      operation,
      (next) => {
        phase = next
        if (!closed) setFollow({ step: 'adding', phase })
      },
      { sleep },
    ).then((outcome) => {
      if (closed) return
      const settled: Follow = outcome.ok
        ? { step: 'running', phase }
        : { step: 'failed', phase, error: outcome.error, logs: outcome.logs }
      SETTLED.set(key, settled)
      setFollow(settled)
    })
    return () => {
      closed = true
    }
  }, [host, key, name, operation])
  return { follow, since: since.current }
}

/** Ticks once a second while `active`: the clock beside an install. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!active) return
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [active])
  return now
}

/** What a running worker registered (functions::list leaves internal ones out). */
function useWorkerFunctions(host: Host, name: string | null): FunctionEntry[] | null {
  const [functions, setFunctions] = useState<FunctionEntry[] | null>(null)
  const asked = useRef<string | null>(null)
  useEffect(() => {
    if (name === null || asked.current === name) return
    asked.current = name
    host.iii.trigger<{ functions: FunctionEntry[] }>('engine::functions::list', {}).then(
      ({ functions: all }) => setFunctions(workerFunctions(all, name)),
      () => setFunctions([]),
    )
  }, [host, name])
  return functions
}

const plural = (count: number, one: string) => `${count} ${one}${count === 1 ? '' : 's'}`

/** A folder by its last two segments, `…/workers/todo-a7`: a narrow card
    keeps the worker's name, which an end ellipsis would cut. */
export function shortFolder(path: string): string {
  const parts = path.split('/').filter(Boolean)
  return parts.length > 2 ? `…/${parts.slice(-2).join('/')}` : path
}

function ScaffoldCard({
  host,
  request,
  result,
  note,
}: {
  host: Host
  request: ScaffoldRequest
  result: ScaffoldResult | null
  note: string | null
}) {
  const name = result?.name ?? request.name ?? 'worker'
  const template = result?.template ?? request.template
  const folder = result?.directory ?? request.directory ?? `workers/${name}`
  const operation = result?.operation_id ?? null
  const { follow, since } = useFollowStart(host, operation ? name : null, operation)
  const now = useNow(follow?.step === 'adding')
  const functions = useWorkerFunctions(host, follow?.step === 'running' ? name : null)
  const entry = result ? entryFile(result.files.map((file) => file.path)) : null
  const elapsed = formatElapsed(now - since)

  const rows: StepRow[] = []
  if (result === null) {
    rows.push({ key: 'files', label: 'Writing files…', state: 'active' })
    if (request.start !== false) {
      rows.push(
        { key: 'install', label: 'Install', state: 'pending' },
        { key: 'start', label: 'Start', state: 'pending' },
      )
    }
  } else {
    rows.push({
      key: 'files',
      label: `Created ${plural(result.files.length, 'file')}`,
      state: 'done',
      detail: entry && host.panels ? <OpenFile host={host} path={entry} directory={result.directory} /> : null,
    })
    if (!operation && !result.start_error) {
      const why = notStartedReason(note) ?? (request.start === false ? 'The call asked for the files only.' : null)
      rows.push({
        key: 'stack',
        label: 'Not added to the stack',
        state: 'skipped',
        note: `${why ? `${why} ` : ''}To run it, ask the agent to add it to the stack.`,
      })
    } else if (result.start_error) {
      rows.push({
        key: 'install',
        label: 'Not added to the stack',
        state: 'failed',
        failure: <StartFailure error={result.start_error} logs={[]} />,
      })
    } else if (follow) {
      const failure = follow.step === 'failed' ? <StartFailure error={follow.error} logs={follow.logs} /> : null
      const [install, start] = stackSteps(follow)
      rows.push(
        { key: 'install', ...install, detail: install.state === 'active' ? elapsed : null, failure },
        { key: 'start', ...start, detail: start.state === 'active' ? elapsed : null, failure },
      )
    }
  }
  const running = follow?.step === 'running'

  return (
    <section className="shui-scaffold" aria-busy={result === null || follow?.step === 'adding' || undefined}>
      <header className="shui-scaffold-head">
        <PackagePlus aria-hidden className="shui-scaffold-icon" />
        <div className="shui-scaffold-heading">
          <span className={result === null ? `shui-scaffold-title ${uiClasses.pulse}` : 'shui-scaffold-title'}>
            {name}
          </span>
          <span className="shui-scaffold-sub" title={folder}>
            {template ? `${template} → ` : ''}
            {shortFolder(folder)}
          </span>
        </div>
      </header>
      <div className="shui-scaffold-body">
        <StepList rows={rows} />
        {running && functions === null ? (
          <p className={`shui-new-worker-note ${uiClasses.pulse}`}>Reading its functions…</p>
        ) : null}
        {running && functions && functions.length > 0 ? (
          <WorkerFunctions
            functions={functions}
            onTry={
              host.chat?.openDraft
                ? (id) =>
                    host.chat?.openDraft?.({ text: `Call ${id} and show me what it returns.`, title: `Try ${id}` })
                : undefined
            }
          />
        ) : null}
        {running && result ? <WorkerLinks host={host} result={result} /> : null}
      </div>
    </section>
  )
}

/** The entry file, relative to the worker, opening in the IDE. */
function OpenFile({ host, path, directory }: { host: Host; path: string; directory: string }) {
  const label = path.startsWith(`${directory}/`) ? path.slice(directory.length + 1) : path
  return (
    <button
      type="button"
      className="shui-scaffold-file"
      title={`Open ${label} in the IDE`}
      onClick={() => host.panels?.open({ pageId: 'ide', context: { type: 'file', path } })}
    >
      {label}
    </button>
  )
}

/** Where a running worker shows itself: its ADE admin page and public page. */
function WorkerLinks({ host, result }: { host: Host; result: ScaffoldResult }) {
  const admin = hasAdePage(result.template) && host.panels
  const publicPage = result.requires.includes('http')
  if (!admin && !publicPage) return null
  return (
    <div className="shui-scaffold-actions">
      {admin ? (
        <Button variant="ghost" size="sm" onClick={() => host.panels?.open({ pageId: result.name })}>
          <LayoutPanelLeft aria-hidden />
          Open admin page
        </Button>
      ) : null}
      {publicPage ? (
        <Button asChild variant="ghost" size="sm">
          <a href={publicPageHref(result.name)} target="_blank" rel="noreferrer">
            <ExternalLink aria-hidden />
            Open public page
          </a>
        </Button>
      ) : null}
    </div>
  )
}

/** Before approval: the steps the call will take, as the card that then
    fills them in shows them. */
function ScaffoldPreview({ request }: { request: ScaffoldRequest }) {
  const folder = request.directory ?? `workers/${request.name ?? '<name>'}`
  const rows: StepRow[] = [
    {
      key: 'files',
      label: (
        <>
          Write files into <span className="shui-scaffold-path">{folder}</span>
        </>
      ),
      state: 'pending',
    },
  ]
  if (request.start === false) {
    rows.push({ key: 'stack', label: 'Not added to the stack: files only', state: 'skipped' })
  } else {
    rows.push(
      {
        key: 'install',
        label: 'Install',
        state: 'pending',
        note: 'Adds it to the stack with compose::add, which installs its dependencies.',
      },
      { key: 'start', label: 'Start', state: 'pending' },
    )
  }
  return (
    <section className="shui-scaffold">
      <header className="shui-scaffold-head">
        <PackagePlus aria-hidden className="shui-scaffold-icon" />
        <div className="shui-scaffold-heading">
          <span className="shui-scaffold-title">Create {request.name ?? 'a worker'}</span>
          <span className="shui-scaffold-sub">from {request.template ?? 'a template'}</span>
        </div>
      </header>
      <div className="shui-scaffold-body">
        <StepList rows={rows} />
      </div>
    </section>
  )
}

/* ── the renderer ───────────────────────────────────────────────────── */

function render(host: Host, message: FunctionTriggerMessage): ReactNode | null {
  if (!isScaffoldFunction(message.functionId) || message.pendingApproval) return null
  const running = !!message.running
  const output = message.output == null ? undefined : unwrapEnvelope(message.output)
  if (message.functionId === LIST_ID) {
    if (running) return <TemplatesCard host={host} result={null} />
    if (isTemplates(output)) return <TemplatesCard host={host} result={output} />
  } else {
    const request = scaffoldRequest(message.input)
    if (running) return <ScaffoldCard host={host} request={request} result={null} note={null} />
    if (isScaffold(output)) {
      return <ScaffoldCard host={host} request={request} result={output} note={harnessNote(message.output)} />
    }
  }
  // Refusals (C230-C235, a gate's denial) keep the shared error card.
  const error = !running && message.output != null ? parseShellErrorDisplay(message.output) : null
  return error ? <ErrorDisplayView display={error} /> : null
}

function renderPreview(message: FunctionTriggerMessage): ReactNode | null {
  if (message.functionId !== SCAFFOLD_ID) return null
  return <ScaffoldPreview request={scaffoldRequest(message.input)} />
}

export function createScaffoldRenderer(host: Host): FunctionTriggerRenderer {
  return {
    id: 'ide/page.js#scaffold',
    isMatch: isScaffoldFunction,
    tryRender: (message) => render(host, message),
    tryRenderRunning: (message) => render(host, message),
    tryRenderPreview: renderPreview,
    FunctionIdLabel,
  }
}

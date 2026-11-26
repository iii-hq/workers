// The one screen every deploy from this machine goes through, as far as the check: its header says the next thing
// to do, You lists what only the person can do (with the action in the item), Your agent lists what it fixes, one
// closed fold holds the rest, and when a check passes "What this deploy does" takes the lists' place. DeployFlow
// puts it in the area and shows the deployment itself.

import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Input,
  Skeleton,
} from '@iii-dev/console-ui'
import { ChevronRight, CircleAlert, CircleDashed, Check as Done, Info, Loader2 } from 'lucide-react'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { AgentControls } from './AgentRow'
import { askedAt, clock, FOLDER_NOTE, useFolderNote, useLaunch } from './asked'
import { ConfigImportDialog } from './ConfigImport'
import { configAction, findings, type Item, location, valueName } from './findings'
import { canOpenChat, handOff, openAgentChat } from './handoff'
import { hubOf, useActions, useChecking } from './hub'
import {
  activeSameKind,
  appFile,
  CHECK_AGAIN,
  cloudPending,
  cloudPendingNote,
  composeFile,
  configDir,
  conflictFiles,
  deployBusy,
  duration,
  envHead,
  envPageUrl,
  fixBranch,
  isDirectEnv,
  isFailedDeploy,
  minutesLabel,
  openProblems,
  opensToPublic,
  plainDetail,
  plainMessage,
  plural,
  postgresNote,
  problemTitle,
  RESOURCE_KIND,
  roleOf,
  sendLabel,
  servingDeploy,
  settingWords,
  sha7,
  signInRequired,
  stageLabel,
  stageText,
  stageTime,
  typicalDeployMs,
  usablePr,
  workingOnIds,
} from './model'
import {
  DetailRow,
  ErrorPanel,
  LinkButton,
  Mono,
  Notice,
  PageAction,
  SectionCard,
  Spinner,
  StatusChip,
  StepHeader,
  useNow,
} from './parts'
import type { Route } from './route'
import type { AgentState, Check, Deploy, Status, ValuesSaved, WorkerError } from './types'
import { useValues, ValuesDialog } from './ValuesEditor'

export type CheckStageProps = {
  host: Host
  status: Status
  go: (r: Route) => void
  narrow: boolean
  /** Setup still passes it until the first-time package drops its check step; the screen does not read it. */
  scenario?: 'setup' | 'change'
  /** The Change panel, open under the header (DeployFlow keeps it), and what its link does. */
  panel?: ReactNode
  onChange?: () => void
}

type Mark = 'ok' | 'bad' | 'work' | 'wait' | 'note'
type Row = {
  key: string
  mark: Mark
  aria?: string
  label: ReactNode
  state: string
  detail?: string | null
  log?: string[]
}
type Act = { label: string; onClick?: () => void; href?: string; disabled?: boolean }
/** A panel for why the check can't finish (or what only the cloud saw), in place of its result. */
type Stop = {
  key: string
  variant: 'info' | 'warn' | 'alert'
  headline: string
  detail: ReactNode
  acts: Act[]
  /** It keeps the deploy button off; the note says why. */
  lock?: string
}

const DOCKER_DOCS = 'https://docs.docker.com/get-started/get-docker/'
const AT_LEAST =
  'iii cloud stops at the first thing it cannot accept, so more can show up after the first round is fixed.'
const UNLOCKS = 'Unlocks when a check passes on the commit you deploy.'
const ACTIVE: AgentState[] = ['starting', 'working', 'waiting_for_you', 'waiting', 'stalled']
const MARKS = {
  ok: [Done, 'done'],
  bad: [CircleAlert, 'problem'],
  work: [Loader2, 'working'],
  wait: [CircleDashed, 'next'],
  note: [Info, 'warning'],
} as const
/** Files `bring_changes` never brings: secrets stay on this machine. */
const unbringable = (path: string) => /(^|\/)\.env[^/]*$/.test(path)

const elapsed = (ms: number) => {
  const t = Math.max(0, Math.round(ms / 1000))
  return t < 60 ? `${t} s` : `${Math.floor(t / 60)} min ${t % 60} s`
}
const agentOn = (s: Status) => !!s.agent && ACTIVE.includes(s.agent.state)
/** The agent is on the check's fixes (a ready or deploy-fix hand-off); stalled stays here. */
const fixing = (s: Status) => agentOn(s) && (s.agent?.kind ?? s.handoff_brief?.kind ?? 'ready') !== 'pull_request'
const github = (s: Status, path: string) => `https://github.com/${s.project?.repository ?? ''}/${path}`

/** `check` for this screen: it runs the local run, pushes the fix branch, keeps to a picked compose file. */
export function checkPayload(s: Status, manifest = appFile(s).path): Record<string, unknown> {
  const switches = manifest !== composeFile(s.project?.environment?.manifest)
  return { run: true, ...(fixBranch(s) ? { push: true } : {}), ...(switches ? { manifest } : {}) }
}

function MarkIcon({ mark, label }: { mark: Mark; label?: string }) {
  const [Icon, word] = MARKS[mark]
  return (
    <Icon
      className={mark === 'work' ? 'iii-cloud-mark iii-ui-spin' : 'iii-cloud-mark'}
      data-mark={mark}
      role="img"
      aria-label={label ?? word}
    />
  )
}

/* ── Where the page is: the header ─────────────────────────────────── */

/** The title (the next thing to do), what goes, and the one primary action: on the right, or in a bar on a phone. */
export function ScreenHead({
  title,
  chip,
  sub,
  action,
  note,
  narrow,
}: {
  title: string
  chip?: ReactNode
  sub?: ReactNode
  action?: ReactNode
  note?: ReactNode
  narrow: boolean
}) {
  return (
    <StepHeader
      title={title}
      chip={chip ? <span aria-live="polite">{chip}</span> : null}
      sub={sub}
      forward={narrow ? null : action}
      reason={narrow ? null : note}
    />
  )
}

/** A phone's primary action, at the bottom of the pane. */
export function ActionBar({ narrow, action, note }: { narrow: boolean; action?: ReactNode; note?: ReactNode }) {
  if (!narrow || !action) return null
  return (
    <div className="iii-cloud-cs-bar">
      {action}
      {note ? <span className="iii-cloud-cs-barnote">{note}</span> : null}
    </div>
  )
}

/** "feat/search-rank: 3 commits and your 2 uncommitted files, to dev": what goes, from what the check and the folder carry. */
function goesLine(s: Status, c: Check | null, env: string): string {
  const p = s.project
  const source = p?.workspace?.source ?? p?.checkout.branch ?? (p ? sha7(p.checkout.head) : 'your folder')
  const brought = !!c?.commits?.some((x) => x.message.startsWith('Uncommitted changes from '))
  // Before a check, the folder's own files are what the check will take.
  const files = !c || brought ? (p?.checkout.changed_files.filter((f) => !unbringable(f)).length ?? 0) : 0
  const list = c?.commits?.length ? c.commits : p?.checkout.not_served
  const commits = list ? list.length - (brought ? 1 : 0) : null
  const parts = [commits ? plural(commits, 'commit') : null, files ? `your ${plural(files, 'uncommitted file')}` : null]
    .filter(Boolean)
    .join(' and ')
  return `${source}: ${parts || `nothing ${env} doesn't have`}, to ${env}`
}

/** The line under every title: what goes, the compose file, and "Change". */
export function GoesSub({ s, c, env, onChange }: { s: Status; c: Check | null; env: string; onChange?: () => void }) {
  return (
    <p className="iii-cloud-cs-sub">
      {goesLine(s, c, env)} · <Mono>{appFile(s).path}</Mono>
      {onChange ? (
        <>
          {' · '}
          <button type="button" className="iii-cloud-linkbtn iii-cloud-small" onClick={onChange}>
            Change
          </button>
        </>
      ) : null}
    </p>
  )
}

/* ── Calls the screen makes and what a refused one says ────────────── */

/** The codes a check start answers when a merge stands in the way: the agent resolves it in its copy. */
export const MERGE_CODES = new Set(['MERGE_CONFLICT', 'MERGE_IN_PROGRESS'])

/** `MERGE_CONFLICT` (bring the commits in) or `MERGE_IN_PROGRESS` (finish the open merge), with the hand-off. */
export function MergeConflict({ host, status, error }: { host: Host; status: Status; error: WorkerError }) {
  const launching = useLaunch()?.kind === 'launching'
  const open = error.code === 'MERGE_IN_PROGRESS'
  const files = conflictFiles(error.message)
  const ask = () =>
    void handOff(
      host,
      status,
      'ready',
      undefined,
      undefined,
      open
        ? 'Finish the merge in the worktree (commit it), then check again.'
        : `Merge ${status.project?.checkout.branch ?? 'your branch'} into the worktree and resolve the conflicts${files.length ? ` in ${files.join(', ')}` : ''}, then check again.`,
    )
  return (
    <ErrorPanel
      error={error}
      variant="warn"
      action={
        <Button size="sm" variant="primary" disabled={!status.check || !host.chat?.compose || launching} onClick={ask}>
          {open ? 'Ask your agent to finish the merge' : 'Ask your agent to bring your commits in'}
        </Button>
      }
    />
  )
}

/** What a call refused, and the way on. */
export function ActionError({
  host,
  status,
  error,
  checkAgain,
  go,
}: {
  host: Host
  status: Status
  error: WorkerError
  checkAgain: () => void
  go: (r: Route) => void
}) {
  const checking = useChecking(host)
  if (MERGE_CODES.has(error.code)) return <MergeConflict host={host} status={status} error={error} />
  if (CHECK_AGAIN.has(error.code)) {
    return (
      <ErrorPanel
        error={error}
        variant="warn"
        action={
          <Button size="sm" variant="primary" disabled={checking} onClick={checkAgain}>
            Check again
          </Button>
        }
      />
    )
  }
  if (error.code === 'PUSH_STILL_RUNNING' || error.code === 'PUSH_UNKNOWN') {
    // PUSH_UNKNOWN is worded in model.ts (its message ends with git's own words, never shown); the
    // page keeps only the file switch it names. PUSH_STILL_RUNNING is the worker's own sentence.
    const stays = error.message.match(/\s(\S+ stays on \S+?)\.?$/)?.[1]
    return (
      <Notice
        variant="warn"
        headline={plainMessage(error)}
        detail={stays ? `${stays}.` : plainDetail(error)}
        action={
          <Button
            size="sm"
            variant="secondary"
            onClick={() => go({ at: 'env', env: status.project?.environment?.id ?? '' })}
          >
            See Deployments
          </Button>
        }
      />
    )
  }
  return <ErrorPanel error={error} />
}

/**
 * Production's confirm: the person types the environment's name, so a deploy there is never one stray click.
 * It stays open, its confirm disabled, until the write's promise settles: a second click sends nothing.
 */
export function TypedConfirm({
  open,
  onOpenChange,
  name,
  title,
  description,
  confirmLabel,
  onConfirm,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  name: string
  title: string
  description: string
  confirmLabel: string
  onConfirm: () => Promise<void>
}) {
  const [typed, setTyped] = useState('')
  const [busy, setBusy] = useState(false)
  const close = (next: boolean) => {
    if (busy) return
    setTyped('')
    onOpenChange(next)
  }
  const confirm = async () => {
    setBusy(true)
    try {
      await onConfirm()
    } finally {
      setBusy(false)
      setTyped('')
      onOpenChange(false)
    }
  }
  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent>
        <DialogTitle>{title}</DialogTitle>
        <DialogDescription>{description}</DialogDescription>
        <div className="iii-cloud-cs-typed">
          <span>
            Type <Mono>{name}</Mono> to confirm
          </span>
          <Input
            aria-label={`Type ${name} to confirm`}
            value={typed}
            onChange={setTyped}
            autoComplete="off"
            spellCheck={false}
          />
        </div>
        <div className="iii-cloud-cs-dialogfoot">
          <DialogClose asChild>
            <Button variant="secondary" disabled={busy}>
              Cancel
            </Button>
          </DialogClose>
          <Button variant="primary" disabled={busy || typed.trim() !== name} onClick={() => void confirm()}>
            {busy ? (
              <>
                <Spinner />
                {confirmLabel}
              </>
            ) : (
              confirmLabel
            )}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

/* ── The check's two parts, as rows ────────────────────────────────── */

type Part = { status: string; rows: Row[]; foot?: string }
const skeleton = (status: string, n: number): Part => ({
  status,
  rows: Array.from({ length: n }, (_, i) => ({
    key: `s${i}`,
    mark: 'wait' as const,
    label: <Skeleton className="iii-cloud-cs-skel" />,
    state: '',
  })),
})

/** "iii cloud's plan": a row per cloud problem; once the plan is clean, what it runs and the console settings. */
function cloudPart(s: Status, c: Check): Part {
  const plan = c.plan
  if (cloudPending(c)) {
    return {
      status: plan.status === 'unavailable' ? 'unavailable' : 'check pending',
      rows: [{ key: 'pending', mark: 'wait', label: cloudPendingNote(s) ?? '', state: '' }],
    }
  }
  const rows: Row[] = openProblems(s)
    .filter((p) => p.source === 'cloud_plan')
    .map((p) => ({
      key: p.id,
      mark: 'bad',
      label: problemTitle(p),
      state: p.owner === 'person' ? 'needs you' : 'needs your agent',
    }))
  if (plan.status === 'clean') {
    const sum = plan.summary
    const n = plan.confirmations.overrides.length
    rows.push(
      {
        key: 'summary',
        mark: 'ok',
        label: sum ? 'What the plan runs' : `iii cloud's plan accepts ${sha7(c.commit)}`,
        state: sum ? `${plural(sum.workers, 'worker')} in ${plural(sum.groups.length, 'group')}` : 'clean',
      },
      {
        key: 'overrides',
        mark: n ? 'bad' : 'ok',
        label: 'Console settings match your compose file',
        state: n ? `${n} ${n === 1 ? 'differs' : 'differ'}` : 'none differ',
      },
    )
  }
  return {
    status:
      plan.status === 'clean'
        ? `done · ${duration(c.duration_ms)}`
        : `${plan.blocking} to fix${plan.at_least ? ', at least' : ''}`,
    rows,
    foot: plan.at_least ? AT_LEAST : undefined,
  }
}

/** "Local run": the run's seven stages, live through the run events. */
function runPart(s: Status, c: Check, now: number, running: boolean): Part {
  const r = c.run
  if (!r) {
    return running
      ? skeleton('starting', 5)
      : {
          status: 'not run yet',
          rows: [{ key: 'none', mark: 'wait', label: "The local run hasn't started", state: '' }],
        }
  }
  const postgres = !!postgresNote(s)
  const rows: Row[] = r.stages.map((st) => {
    const t = stageText(r, st, postgres)
    return {
      key: st.name,
      mark: st.status === 'passed' ? 'ok' : st.status === 'failed' ? 'bad' : st.status === 'running' ? 'work' : 'wait',
      label: (
        <>
          {t.label}
          {t.mono ? <Mono>{t.mono}</Mono> : null}
        </>
      ),
      state: st.status === 'pending' ? 'waiting' : st.status === 'running' ? 'running' : stageTime(r, st),
      detail: t.note,
      log: st.status === 'failed' ? r.log_tail : undefined,
    }
  })
  const start = Date.parse(r.started_at)
  return {
    status:
      r.status === 'running'
        ? `running · ${elapsed(now - start)}`
        : r.status === 'passed'
          ? r.reused
            ? 'the local run of this commit already passed'
            : `passed${r.finished_at ? ` · ${duration(Date.parse(r.finished_at) - start)}` : ''}`
          : `failed${r.error ? ` at ${stageLabel(r.error.stage)}` : ''}`,
    rows,
    foot: [
      `Engine ${r.engine}.`,
      r.status === 'passed' && r.not_covered.length
        ? `Not covered by the local run: ${r.not_covered.join(', ')}.`
        : null,
    ]
      .filter(Boolean)
      .join(' '),
  }
}

function Rows({ rows }: { rows: Row[] }) {
  return (
    <ul className="iii-cloud-cs-rows">
      {rows.map((r) => (
        <li key={r.key} className="iii-cloud-cs-row" data-mark={r.mark}>
          <MarkIcon mark={r.mark} label={r.aria} />
          <span className="iii-cloud-cs-label">{r.label}</span>
          <span className="iii-cloud-cs-state">{r.state}</span>
          {r.detail ? <span className="iii-cloud-cs-detail">{r.detail}</span> : null}
          {r.log?.length ? (
            <details className="iii-cloud-cs-log">
              <summary>{`Last lines of ${r.aria ?? 'the local run'}`}</summary>
              <pre>{r.log.join('\n')}</pre>
            </details>
          ) : null}
        </li>
      ))}
    </ul>
  )
}

/* ── You ───────────────────────────────────────────────────────────── */

type Override = Check['plan']['confirmations']['overrides'][number]

/** A missing secret, typed where it is asked: the field and Save in the item itself. The value goes only to `values::save`. */
function SecretField({
  name,
  env,
  why,
  save,
}: {
  name: string
  env: string
  why: string | null
  /** The error to show, or null once it is saved. No value: left empty on purpose. */
  save: (value: string | null) => Promise<string | null>
}) {
  const [value, setValue] = useState('')
  const [saved, setSaved] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const submit = async (typed: string | null) => {
    setBusy(true)
    const out = await save(typed)
    setBusy(false)
    setError(out)
    if (out === null) {
      setValue('')
      setSaved(true)
    }
  }
  return (
    <li className="iii-cloud-cs-need">
      <span className="iii-cloud-cs-label">
        <Mono>{name}</Mono> {`has no value in ${env}`}
      </span>
      <span className="iii-cloud-cs-detail">
        {[why, `Saved to ${env} only; your agent never sees it.`].filter(Boolean).join(' ')}
      </span>
      {saved ? (
        <span className="iii-cloud-cs-saved">
          <MarkIcon mark="ok" label="saved" />
          Saved. The check runs again.
        </span>
      ) : (
        <span className="iii-cloud-cs-field">
          <Input
            aria-label={`Value of ${name}`}
            type="password"
            autoComplete="off"
            placeholder="Paste the secret"
            value={value}
            onChange={setValue}
          />
          <Button variant="secondary" disabled={!value || busy} onClick={() => void submit(value)}>
            Save
          </Button>
          <Button variant="ghost" disabled={busy} onClick={() => void submit(null)}>
            Leave it empty
          </Button>
        </span>
      )}
      {error ? <span className="iii-cloud-cs-err">{error}</span> : null}
    </li>
  )
}

/** A console setting that differs from the compose file: the two ways on, side by side, each with its effect. */
function DecisionRow({
  o,
  env,
  file,
  canAsk,
  pick,
}: {
  o: Override
  env: string
  file: string
  canAsk: boolean
  pick: (choice: 'file' | 'console') => void
}) {
  const opens = opensToPublic(o)
  // Making a service public is the one choice that deserves a second look: the console's value is the safe one.
  const best = opens ? 'console' : 'file'
  return (
    <li className="iii-cloud-cs-need">
      <span className="iii-cloud-cs-label">{`${env}'s ${o.setting} was changed in iii cloud's console`}</span>
      <span className="iii-cloud-cs-detail">
        The console says <Mono>{settingWords(o.setting, o.console)}</Mono>. Your compose file says{' '}
        <Mono>{settingWords(o.setting, o.file)}</Mono>. Pick the one this deploy runs.
      </span>
      <span className="iii-cloud-cs-opts">
        <button
          type="button"
          className="iii-cloud-cs-opt"
          data-best={best === 'file' ? '' : undefined}
          onClick={() => pick('file')}
        >
          <b>Use my compose file</b>
          <span>
            {best === 'file' ? 'Recommended. ' : ''}
            {`The console's value is removed, and ${file} decides from now on.${opens ? ' It makes the service public.' : ''}`}
          </span>
        </button>
        <button
          type="button"
          className="iii-cloud-cs-opt"
          data-best={best === 'console' ? '' : undefined}
          disabled={!canAsk}
          onClick={() => pick('console')}
        >
          <b>Keep the console's value</b>
          <span>
            {best === 'console' ? 'Recommended. ' : ''}
            {`Your agent changes ${file} to match the console, then the check runs again.`}
          </span>
        </button>
      </span>
    </li>
  )
}

/** Any other problem only the person can solve: what happened, why it matters, what to do. Never a bare path. */
function PlainNeed({ item: i }: { item: Item }) {
  const loc =
    i.line == null && !i.target && i.file && i.detail?.includes(i.file) ? null : location(i.file, i.line, i.target)
  return (
    <li className="iii-cloud-cs-need">
      <span className="iii-cloud-cs-label">{i.title}</span>
      {i.detail ? <span className="iii-cloud-cs-detail">{i.detail}</span> : null}
      {i.fix ? <span className="iii-cloud-cs-detail">{i.fix}</span> : null}
      {loc ? <span className="iii-cloud-cs-loc">{loc}</span> : null}
    </li>
  )
}

/* ── Your agent ────────────────────────────────────────────────────── */

type Line = { key: string; title: string; detail: string | null; id: string | null }

/* ── Details ───────────────────────────────────────────────────────── */

type Past = { id: string; commit: string; checked_at: string; ready: boolean; problems: number; duration_ms: number }
/** The checks of the agent's hand-off, oldest first, from the one it was made for (its brief's check first). */
function historyOf(s: Status, c: Check | null, envId: string): Past[] {
  const hid = s.agent?.handoff_id
  if (!hid) return []
  const brief = s.handoff_brief?.handoff_id === hid ? s.handoff_brief : null
  const list: Past[] = (s.checks ?? [])
    .filter((x) => x.environment_id === envId && (x.handoff_id === hid || x.id === brief?.check_id))
    .reverse()
  // The check event arrives before the next status read lists it.
  if (c && c.handoff_id === hid && !list.some((x) => x.id === c.id)) {
    list.push({ ...c, problems: openProblems(s).length })
  }
  return list
}

function Details({ summary, children }: { summary: string; children: ReactNode }) {
  const [open, setOpen] = useState(false)
  return (
    <CollapsibleCard
      open={open}
      onOpenChange={setOpen}
      role="region"
      aria-label="Details"
      className="iii-cloud-cs-panel"
    >
      <CollapsibleCardTrigger className="iii-cloud-cs-head" aria-expanded={open}>
        <ChevronRight className="iii-cloud-cs-chev" aria-hidden />
        <span className="iii-cloud-cs-title">Details</span>
        <span className="iii-cloud-cs-note">{summary}</span>
      </CollapsibleCardTrigger>
      <CollapsibleCardContent>{children}</CollapsibleCardContent>
    </CollapsibleCard>
  )
}

function Group({ title, count, children }: { title: string; count: number; children: ReactNode }) {
  return (
    <section className="iii-cloud-cs-group" aria-label={title}>
      <h3>
        {title} <span className="iii-cloud-count">{count}</span>
      </h3>
      <ul className="iii-cloud-cs-rows">{children}</ul>
    </section>
  )
}

function Fact({ mark, title, sub, end }: { mark: Mark; title: ReactNode; sub?: ReactNode; end?: ReactNode }) {
  return (
    <li className="iii-cloud-cs-item" data-mark={mark}>
      <MarkIcon mark={mark} />
      <div className="iii-cloud-cs-body">
        <span className="iii-cloud-cs-label">{title}</span>
        {sub ? <span className="iii-cloud-cs-detail">{sub}</span> : null}
        {end}
      </div>
    </li>
  )
}

/* ── Ready: what this deploy does ──────────────────────────────────── */

function Ready({ s, c, host }: { s: Status; c: Check; host: Host }) {
  const p = s.project
  const env = p?.environment
  const launching = useLaunch()?.kind === 'launching'
  if (!p || !env) return null
  const direct = isDirectEnv(s, env)
  const pr = direct ? null : usablePr(s)
  const file = appFile(s)
  const { resources, overrides } = c.plan.confirmations
  const role = roleOf(s)
  const serving = servingDeploy(s, env.id)
  const typical = typicalDeployMs(s, env.id)
  const branch = p.workspace?.source ?? p.checkout.branch
  // Whose commits (newest first): the agent's down to `from_head`, the checkout's from it. No `from_head`: all the agent's.
  const commits = c.commits ?? []
  const from = p.workspace?.from_head
  const at = from ? commits.findIndex((x) => x.sha === from) : -1
  const agent = !from ? commits.length : at >= 0 ? at : from === c.base ? commits.length : null
  const brought = commits.some((x) => x.message.startsWith('Uncommitted changes from '))
  const files = brought ? p.checkout.changed_files.filter((f) => !unbringable(f)).length : 0
  const mine = agent === null ? 0 : commits.length - agent
  // The commit that carries the uncommitted files is the folder's too, said apart.
  const own = mine - (brought ? 1 : 0)
  const whose =
    agent === null
      ? null
      : [
          own > 0 ? `${own} of yours` : null,
          brought && files ? `1 with your ${plural(files, 'uncommitted file')}` : null,
          agent ? `${plural(agent, 'fix', 'fixes')} by your agent` : null,
        ]
          .filter(Boolean)
          .join(', ')
  const release = c.run?.stages.find((x) => x.name === 'release' && x.status === 'passed' && x.detail)
  const sum = c.plan.summary
  const since = [...(s.checks ?? [])]
    .reverse()
    .find((x) => x.environment_id === env.id && x.ready && x.id !== c.id)?.checked_at
  const saved = s.secrets.filter(
    (r) =>
      r.environment_id === env.id &&
      r.state === 'set' &&
      !!r.resolved_at &&
      (!since || Date.parse(r.resolved_at) >= Date.parse(since)),
  )
  const fixes = agent && branch ? `your agent's ${plural(agent, 'fix', 'fixes')}` : null
  const filesText = files ? `your ${plural(files, 'uncommitted file')}` : null
  const goes = [fixes, filesText].filter(Boolean).join(' and ')
  return (
    <>
      <SectionCard title="What this deploy does" flush>
        <DetailRow
          label="Code"
          note={whose}
          action={
            <LinkButton href={github(s, c.base ? `compare/${c.base}...${c.commit}` : `commit/${c.commit}`)}>
              See changes
            </LinkButton>
          }
        >
          {c.commits
            ? commits.length
              ? `${plural(commits.length, 'commit')} ${env.name} doesn't have`
              : `Nothing ${env.name} doesn't have`
            : `Commit ${sha7(c.commit)}`}
        </DetailRow>
        <DetailRow
          label="Compose file"
          note={file.note}
          action={<LinkButton href={github(s, `blob/${c.commit}/${file.path}`)}>See the file</LinkButton>}
        >
          <Mono>{file.path}</Mono>
          {sum ? `: ${plural(sum.workers, 'worker')} in ${plural(sum.groups.length, 'group')}` : ''}
          {overrides.length ? `. Console settings now come from it: ${overrides.map((o) => o.setting).join(', ')}` : ''}
        </DetailRow>
        {resources.length ? (
          <DetailRow
            label="Database"
            note={
              env.production
                ? `Approving resources in ${env.name} takes Owner or Admin. You are ${role ?? 'a member'}.`
                : null
            }
          >
            {resources.map((r) => (
              <span key={`${r.kind}:${r.alias}`} className="iii-cloud-cs-line">
                Creates a shared {RESOURCE_KIND[r.kind] ?? r.kind} for <Mono>{r.binding ?? r.alias}</Mono>
              </span>
            ))}
          </DetailRow>
        ) : null}
        {saved.length ? (
          <DetailRow label="Secrets" note={`Saved by you at ${clock(saved[0].resolved_at as string)}`}>
            {saved.length} new: <Mono>{saved.map((r) => r.name).join(', ')}</Mono>
          </DetailRow>
        ) : null}
        {release ? (
          <DetailRow label="Release command" note="It ran in the local run, and runs before the new version starts.">
            <Mono>{release.detail}</Mono>
          </DetailRow>
        ) : null}
        {goes && branch ? (
          <DetailRow label="Your branch" note="The next deploy starts from it.">
            {`After the deploy, ${branch} gets ${goes}${fixes && filesText ? ' as one commit' : ''}`}
          </DetailRow>
        ) : null}
        {direct ? null : (
          <DetailRow
            label="How it ships"
            note={[
              pr
                ? `${pr.head} → ${pr.base}, opened by your agent. GitHub checks: ${pr.checks}.${pr.main_moved ? '' : ` ${pr.base} has not moved since the check.`}`
                : 'Your agent opens a pull request; merging it here deploys.',
            ].join(' ')}
            action={
              pr ? (
                <LinkButton href={pr.url}>Open on GitHub</LinkButton>
              ) : activeSameKind(s, 'pull_request') ? (
                <Button size="sm" variant="secondary" onClick={() => openAgentChat(host, s)}>
                  Open chat
                </Button>
              ) : (
                <Button
                  size="sm"
                  variant="primary"
                  disabled={launching || !host.chat?.compose}
                  onClick={() => void handOff(host, s, 'pull_request')}
                >
                  {sendLabel(s, 'pull_request', 'Ask your agent to open it')}
                </Button>
              )
            }
          >
            {pr ? `Merges pull request #${pr.number} into ${pr.base}, then deploys` : `Nothing to merge yet`}
          </DetailRow>
        )}
      </SectionCard>
      <p className="iii-cloud-cs-foot">
        {[
          serving
            ? `${env.name} serves #${serving.number} until the new deployment is live.`
            : `Nothing is live on ${env.name} yet.`,
          typical ? `It takes about ${minutesLabel(typical)}.` : null,
          "Only you can confirm this; your agent can't.",
          pr || direct
            ? 'Pinned to the plan the check saw: if it plans differently, nothing is deployed and you see why.'
            : null,
        ]
          .filter(Boolean)
          .join(' ')}
      </p>
      {env.production && role && !/^(owner|admin)$/i.test(role) ? (
        <Notice
          variant="warn"
          headline={`Deploying to ${env.name} takes Owner or Admin`}
          detail={`You are ${role}. iii cloud decides; if it refuses, its answer shows here.`}
        />
      ) : null}
    </>
  )
}

/* ── The screen ────────────────────────────────────────────────────── */

export function CheckStage(props: CheckStageProps) {
  const { host, status: s, go, narrow, panel, onChange } = props
  const hubChecking = useChecking(host)
  const now = useNow(1000)
  const { error, busy, call } = useActions(host)
  const folder = useFolderNote(s.check?.id) ? FOLDER_NOTE : undefined
  const launching = useLaunch()?.kind === 'launching'
  const [confirming, setConfirming] = useState(false)
  const [importing, setImporting] = useState(false)
  // The check this page saw running: when it ends not ready, the agent may start (once).
  const watch = useRef<{ base: string | null } | null>(null)

  const c = s.check
  const run = c?.run ?? null
  const running = hubChecking || run?.status === 'running'
  // While a check is in flight and nothing of it has arrived, the shown check is the previous one.
  const fresh = !hubChecking || run?.status === 'running'
  const auto = s.auto_fix ?? true
  const p = s.project
  const envId = c?.environment_id ?? p?.environment?.id ?? ''
  const envRow = p?.environments.find((e) => e.id === envId)
  const env = envRow?.name ?? p?.environment?.name ?? 'the environment'
  const source = p?.workspace?.source ?? p?.checkout.branch ?? (p ? sha7(p.checkout.head) : 'your folder')

  // The check starts from this screen's own buttons; the agent starts by itself when the check it watched ends with
  // agent-owned problems, if auto_fix is on. ponytail: only while the page is open; the worker's `fix` covers a closed page.
  useEffect(() => {
    if (running) {
      watch.current ??= { base: fresh ? null : (c?.id ?? null) }
      return
    }
    const w = watch.current
    if (!w || !c || c.id === w.base) return
    watch.current = null
    if (!auto || c.ready || c.handoff_id || agentOn(s) || askedAt(c.id)) return
    if (openProblems(s).some((x) => x.owner === 'agent')) void handOff(host, s, 'ready')
  })

  const checkAgain = () => void call('check', 'iii-cloud::check', checkPayload(s))
  const ask = () => void handOff(host, s, 'ready')
  const canAsk = !!host.chat?.compose && !launching

  const first = !s.deploys.some((d) => d.environment_id === envId)
  const values = useValues(host, s, first && c ? { manifest: c.manifest || null } : null)
  const [editing, setEditing] = useState(false)

  if (!c && !running) {
    return (
      <div className="iii-cloud-cs" data-narrow={narrow ? '' : undefined}>
        <ScreenHead
          title={`Check ${source} for ${env}`}
          sub={<GoesSub s={s} c={null} env={env} onChange={onChange} />}
          action={<PageAction onClick={() => undefined} disabled>{`Deploy to ${env}`}</PageAction>}
          note={UNLOCKS}
          narrow={narrow}
        />
        {panel}
        {error ? <ActionError host={host} status={s} error={error} checkAgain={checkAgain} go={go} /> : null}
        <Notice
          variant="info"
          headline="Not checked yet"
          detail="A check runs iii cloud's plan and the local run on this commit. Nothing is deployed."
          action={
            <Button variant="primary" onClick={checkAgain}>
              Check
            </Button>
          }
        />
        <ActionBar
          narrow={narrow}
          action={<PageAction onClick={() => undefined} disabled>{`Deploy to ${env}`}</PageAction>}
          note={UNLOCKS}
        />
      </div>
    )
  }

  const chosen = s.override_choices ?? {}
  const f = findings(s)
  const open = openProblems(s)

  // The two lists the title counts: what only the person can do, and what the agent fixes.
  // The first time: names not in iii cloud that a file of the folder (or another environment) can fill are one row,
  // opened in the bulk editor; only the names nothing can fill keep a row each.
  const read = values.data
  const fillable =
    read?.save === 'ok'
      ? read.items.filter(
          (i) =>
            i.in_cloud === 'unset' &&
            !i.database &&
            !i.invalid_name &&
            i.suggestions.some((x) => x.source === 'env_file' || x.source === 'copy'),
        )
      : []
  const fillableNames = new Set(fillable.map((i) => i.name))
  const fillFiles = [
    ...new Set(
      fillable.flatMap((i) =>
        i.suggestions.flatMap((x) => (x.source === 'env_file' ? [x.file.split('/').at(-1)] : [])),
      ),
    ),
  ]
  const needItems = f.person.filter(
    (i) => !i.fixed && i.severity === 'block' && !configAction(i) && !fillableNames.has(valueName(i) ?? ''),
  )
  const decisions = c ? c.plan.confirmations.overrides.filter((o) => chosen[o.setting] !== 'file') : []
  const decide = decisions.filter((o) => chosen[o.setting] !== 'console')
  const fromFile = c ? c.plan.confirmations.overrides.filter((o) => chosen[o.setting] === 'file') : []
  const agentItems = f.agent.filter((i) => !i.fixed && i.severity === 'block')
  const lines: Line[] = [
    ...agentItems.map((i) => ({ key: i.key, title: i.title, detail: i.detail ?? i.fix, id: i.problem?.id ?? null })),
    ...decisions
      .filter((o) => chosen[o.setting] === 'console')
      .map((o) => ({
        key: `override:${o.setting}`,
        title: `Making ${appFile(s).path} match the console's ${o.setting}`,
        detail: `You asked your agent to keep the console's value: ${settingWords(o.setting, o.console)}.`,
        id: null,
      })),
  ]
  // The agent fixing a failed deployment: the passed check is the failed deployment's, not a go-ahead.
  const deployFix = agentOn(s) && (s.agent?.kind ?? s.handoff_brief?.kind) === 'deploy_fix'
  const failedDeploy = deployFix ? s.deploys.find((d) => d.environment_id === envId && isFailedDeploy(d)) : undefined
  if (failedDeploy && !lines.length) {
    lines.push({
      key: failedDeploy.id,
      title: failedDeploy.failure?.title ?? `Deployment #${failedDeploy.number} failed`,
      detail: failedDeploy.failure?.hint ?? failedDeploy.error?.hint ?? null,
      id: null,
    })
  }
  const needN = needItems.length + decide.length + (fillable.length ? 1 : 0)
  const ready = !!c?.ready && !running && !deployFix
  const working = fixing(s) && !ready
  const a = s.agent

  const direct = !!p?.environment && isDirectEnv(s, p.environment)
  const pr = direct ? null : usablePr(s)
  const production = !!p?.environment?.production
  const deploy = async () => {
    const d = await call<Deploy>(
      'deploy',
      'iii-cloud::deploy',
      pr ? { check_id: c?.id, pr: pr.number } : { check_id: c?.id },
    )
    if (!d) return
    // The screen follows the deployment, so the status has to list it first.
    await hubOf(host).refresh(false)
    go({ at: 'deploy', env: envId, deploy: d.id })
  }
  const startDeploy = () => (production ? setConfirming(true) : void deploy())

  // The warnings: what won't block, from either owner, the notes and the skipped names.
  const warnings: Item[] = [
    ...f.person.filter((i) => !i.fixed && !needItems.includes(i)),
    ...f.agent.filter((i) => !i.fixed && !agentItems.includes(i)),
    ...f.notes,
    ...f.skipped,
  ]
  const fixed = [...f.person, ...f.agent].filter((i) => i.fixed)
  const past = historyOf(s, c, envId)

  const stops: Stop[] = []
  if (c) {
    if (s.connection.status === 'expired' || signInRequired(s)) {
      stops.push({
        key: 'signin',
        variant: 'alert',
        headline: 'Your iii cloud sign-in expired',
        detail: `${run?.status === 'passed' ? 'The local run finished and passed. ' : ''}iii cloud's plan needs you to sign in again; nothing else is lost.`,
        acts: [
          { label: 'Sign in again', onClick: () => void call('connect', 'iii-cloud::connect', { method: 'browser' }) },
        ],
        lock: 'Sign in again first.',
      })
    }
    // The source the worktree took (a branch, else the checkout's HEAD) has moved since: an older worker says nothing.
    const ws = p?.workspace
    const taken = ws?.mode === 'worktree' ? ws.source_sha : null
    const tipHere =
      !ws?.source || ws.source === p?.checkout.branch
        ? p?.checkout.head
        : p?.checkout.branches?.find((b) => b.name === ws.source)?.sha
    if (taken && tipHere && sha7(tipHere) !== sha7(taken)) {
      // ponytail: the workspace doesn't record `bring_changes`; its commit's message says it was used.
      const brought = !!c.commits?.some((x) => x.message.startsWith('Uncommitted changes from '))
      const useCheckout = async () => {
        const used = await call('use', 'iii-cloud::project::use', {
          path: p?.path,
          environment_id: envId,
          mode: ws?.mode,
          from: 'checkout',
          ...(ws?.source ? { source: ws.source } : {}),
          ...(brought ? { bring_changes: true } : {}),
        })
        if (used !== null) checkAgain()
      }
      stops.push({
        key: 'outdated',
        variant: 'info',
        headline: `${source} moved after the check`,
        detail: `${sha7(c.commit)} ${c.ready ? 'passed' : 'was checked'}, but ${source} now ends at ${sha7(tipHere)}. Only a commit that passed can be deployed.`,
        acts: [
          { label: `Check ${sha7(tipHere)}`, onClick: () => void useCheckout(), disabled: running },
          ...(c.ready ? [{ label: `Deploy ${sha7(c.commit)} without it`, onClick: startDeploy }] : []),
        ],
        lock: `Check ${sha7(tipHere)} first.`,
      })
    }
    const err = run?.status === 'failed' ? run.error : null
    if (err?.code === 'DOCKER_MISSING') {
      const cloudN = open.filter((x) => x.source === 'cloud_plan').length
      stops.push({
        key: 'docker',
        variant: 'alert',
        headline: "Docker isn't running on this machine",
        detail: `The local run starts a database and your workers in containers.${cloudPending(c) ? '' : ` iii cloud's plan still ran: ${cloudN ? plural(cloudN, 'problem') : 'no problems'}.`}`,
        acts: [
          { label: 'Run again', onClick: checkAgain, disabled: running },
          { label: 'How to start Docker', href: DOCKER_DOCS },
        ],
      })
    } else if (err && err.code !== 'RUN_REPLACED' && !open.some((x) => x.source === 'check_here')) {
      stops.push({
        key: 'run',
        variant: 'alert',
        headline: `The local run stopped at ${stageLabel(err.stage)}`,
        detail: plainMessage(err),
        acts: [{ label: 'Run again', onClick: checkAgain, disabled: running }],
      })
    }
    const d = s.deploys.find((x) => x.environment_id === envId)
    if (d?.outcome === 'running' && d.check_id !== c.id) {
      stops.push({
        key: 'busy',
        variant: 'info',
        ...deployBusy(s, d, env),
        acts: [{ label: `Follow #${d.number}`, onClick: () => go({ at: 'deploy', env: envId, deploy: d.id }) }],
        lock: `Unlocks when deployment #${d.number} is done.`,
      })
    } else if (d && d.check_id !== c.id && Date.parse(d.created_at) > Date.parse(c.checked_at)) {
      stops.push({
        key: 'changed',
        variant: 'warn',
        headline: `${env} changed after the check`,
        detail: `Deployment #${d.number} (${sha7(d.commit.sha)}) started after this check. Check on top of it before you deploy.`,
        acts: [{ label: `Check on top of ${sha7(d.commit.sha)}`, onClick: checkAgain, disabled: running }],
        lock: `Check on top of ${sha7(d.commit.sha)} first.`,
      })
    }
    const tip = envHead(s, envId)
    // ponytail: equality only; a check commit that merely contains the tip still warns until the backend reports ancestry.
    if (tip && sha7(tip) !== sha7(c.base) && sha7(tip) !== sha7(c.commit)) {
      stops.push({
        key: 'moved',
        variant: 'warn',
        headline: `The branch ${env} follows has a commit your check doesn't`,
        detail: `${envRow?.branch ?? 'It'} is at ${sha7(tip)} now; the check ran on top of ${sha7(c.base)}. Your commits need to go on top of it.`,
        acts: [
          {
            label: 'Bring it into the check',
            onClick: () =>
              void handOff(
                host,
                s,
                'ready',
                undefined,
                undefined,
                `${envRow?.branch ?? env} moved to ${sha7(tip)}: bring it into the check.`,
              ),
            disabled: !canAsk,
          },
        ],
        lock: `Bring ${sha7(tip)} into the check first.`,
      })
    }
    if (a && (a.state === 'done' || a.state === 'stopped') && !c.ready && !running && lines.length) {
      stops.push({
        key: 'stopped',
        variant: 'warn',
        headline: `Your agent couldn't fix ${plural(lines.length, 'problem')}`,
        detail:
          'It stopped before everything was fixed. Read what it tried in its chat, ask it to finish, or check again.',
        acts: [
          ...(a.session_id && canOpenChat(host)
            ? [{ label: 'Read what it tried', onClick: () => void openAgentChat(host, s) }]
            : []),
          { label: sendLabel(s, 'ready', 'Ask your agent to finish'), onClick: ask, disabled: !canAsk },
          { label: 'Check again', onClick: checkAgain, disabled: running },
        ],
      })
    }
  }
  const lock = stops.find((x) => x.lock)?.lock
  const prLock =
    !direct && ready && (!pr || pr.main_moved)
      ? 'Open the pull request, and check again if the branch moved.'
      : undefined
  const staleDeploy = !!error && CHECK_AGAIN.has(error.code)
  // The local run never ran on a commit iii cloud's plan accepts (a failed run has its own stop).
  const runPending = !!c && !running && c.plan.status === 'clean' && !run && !agentItems.length && !needN && !c.ready

  // The header: the next thing to do, said with the lists' own lengths.
  const title =
    running && !working
      ? `Checking ${source} for ${env}`
      : ready
        ? `Ready to deploy to ${env}`
        : runPending
          ? `iii cloud's plan accepts ${sha7(c?.commit ?? '')}; the local run is left`
          : needN || lines.length || working
            ? [
                needN ? `${plural(needN, 'thing')} ${needN === 1 ? 'needs' : 'need'} you` : null,
                lines.length
                  ? working
                    ? `your agent is fixing ${lines.length}`
                    : `${lines.length} for your agent to fix`
                  : working
                    ? 'your agent is checking its fix'
                    : null,
              ]
                .filter(Boolean)
                .join(' · ')
                .replace(/^./, (ch) => ch.toUpperCase())
            : `Not ready for ${env} yet`
  const chip = running ? (
    <StatusChip live>Checking</StatusChip>
  ) : ready ? (
    <StatusChip tone="success">Passed</StatusChip>
  ) : null
  const ranAt = run?.reused ? c?.checked_at : (run?.finished_at ?? c?.checked_at)
  const sub = ready ? (
    <>
      <p className="iii-cloud-cs-sub">
        <Mono>{c ? sha7(c.commit) : ''}</Mono>
        {` passed iii cloud's plan${run ? ' and the local run' : ''} at ${clock(ranAt ?? '')}${past.length > 1 ? ` · ${past.length} checks` : ''}`}
      </p>
      <GoesSub s={s} c={c} env={env} onChange={onChange} />
    </>
  ) : (
    <GoesSub s={s} c={c} env={env} onChange={onChange} />
  )

  let action: ReactNode
  let note: ReactNode = null
  if (ready || runPending) {
    const off = ready ? (lock ?? prLock) : undefined
    action = ready ? (
      <PageAction disabled={!!off || staleDeploy || busy === 'deploy'} onClick={startDeploy}>
        {direct
          ? `Deploy ${c ? sha7(c.commit) : ''} to ${env}${production ? '…' : ''}`
          : `Merge to ${pr?.base ?? p?.environment?.branch} and deploy to ${env}${production ? '…' : ''}`}
      </PageAction>
    ) : (
      <PageAction disabled={running} onClick={checkAgain}>
        Start the local run
      </PageAction>
    )
    note = off ?? (ready ? "Only you can confirm this; your agent can't." : null)
  } else {
    action = <PageAction onClick={() => undefined} disabled>{`Deploy to ${env}`}</PageAction>
    note = UNLOCKS
  }

  const parts =
    !ready && (running || (!needN && !lines.length))
      ? {
          cloud: c && fresh ? cloudPart(s, c) : skeleton('checking', 4),
          here: c && fresh ? runPart(s, c, now, running) : skeleton('next', 5),
        }
      : null
  const pending = c && !running && !ready ? cloudPendingNote(s) : null
  const secretsOnly =
    (needItems.length > 0 || fillable.length > 0) && !decide.length && needItems.every((i) => valueName(i))
  const youNote = running
    ? 'You can do this while the check runs'
    : secretsOnly
      ? `Secrets go to ${env} only. Your agent never sees them.`
      : "Your agent can't do these"

  // The first-time line: names filled from the folder's .env files, saved as secrets.
  const filled = read?.items.filter((i) => i.here === 'from_file' && i.in_cloud === 'set') ?? []
  const files = [...new Set(filled.map((i) => i.here_file?.split('/').at(-1)).filter(Boolean))]
  const empty = read?.items.filter((i) => i.need === 'optional' && i.in_cloud !== 'set').length ?? 0
  const filledText = filled.length
    ? `${filled.length} of ${read?.items.length} names your compose file reads were filled from ${files.join(' and ')}, saved as secrets.${empty ? ` ${empty} optional ${empty === 1 ? 'one is' : 'ones are'} empty.` : ''}`
    : null

  const saveSecret = async (name: string, value: string | null): Promise<string | null> => {
    const out = await call<ValuesSaved>('save', 'iii-cloud::values::save', {
      environment_id: envId,
      ...(c?.manifest ? { manifest: c.manifest } : {}),
      items: [value === null ? { name, from: 'leave_empty' } : { name, from: 'typed', type: 'secret', value }],
    })
    if (!out) return 'It could not be saved. Try again.'
    const bad = out.results.find((r) => !r.ok)
    if (bad) return bad.error ? plainMessage(bad.error) : `${name} could not be saved. Try again.`
    checkAgain()
    return null
  }
  const choose = (o: Override, choice: 'file' | 'console') => {
    const done = () =>
      call('choose', 'iii-cloud::override::choose', { environment_id: envId, setting: o.setting, choice })
    if (choice === 'file') return void done()
    const keep = `Make ${appFile(s).path}'s x-iii-cloud.${o.setting} match ${env}'s console value ${JSON.stringify(o.console)}, or drop it from the block, so the deploy keeps the console's.`
    void handOff(host, s, 'ready', undefined, undefined, keep).then((sent) => sent && done())
  }

  const current = new Set(workingOnIds(s))
  if (working && !lines.some((l) => l.id && current.has(l.id))) current.clear()
  const leader = working ? (lines.find((l) => l.id && current.has(l.id)) ?? lines[0]) : null
  const idle = a?.last_activity_at ? now - Date.parse(a.last_activity_at) : 0
  const configItem = warnings.find(configAction)
  const lastCheck = past.at(-1)
  const doneN = fixed.length + fromFile.length + (!needN && filledText ? 1 : 0)
  const summary = [
    `Done ${doneN}`,
    `Warnings ${warnings.length}`,
    `Checks ${past.length}${lastCheck ? ` (last ${clock(lastCheck.checked_at)}, ${duration(lastCheck.duration_ms)})` : ''}`,
  ].join(' · ')

  return (
    <div className="iii-cloud-cs" data-narrow={narrow ? '' : undefined}>
      <ScreenHead title={title} chip={chip} sub={sub} action={action} note={note} narrow={narrow} />
      {panel}
      {pending ? <Notice variant="info" headline="iii cloud's plan is pending" detail={pending} /> : null}
      {error ? <ActionError host={host} status={s} error={error} checkAgain={checkAgain} go={go} /> : null}
      {stops.map((x) => (
        <Notice
          key={x.key}
          className="iii-cloud-cs-stop"
          variant={x.variant}
          headline={x.headline}
          detail={x.detail}
          action={
            <span className="iii-cloud-row">
              {x.acts.map((act, i) =>
                act.href ? (
                  <Button key={act.label} asChild size="sm" variant="secondary">
                    <a href={act.href} target="_blank" rel="noreferrer">
                      {act.label}
                    </a>
                  </Button>
                ) : (
                  <Button
                    key={act.label}
                    size="sm"
                    variant={i === 0 ? 'primary' : 'secondary'}
                    disabled={act.disabled}
                    onClick={act.onClick}
                  >
                    {act.label}
                  </Button>
                ),
              )}
            </span>
          }
        />
      ))}

      {needN ? (
        <SectionCard
          title="You"
          count={needN}
          note={youNote}
          end={
            secretsOnly ? (
              <button
                type="button"
                className="iii-cloud-linkbtn iii-cloud-small"
                onClick={() => go({ at: 'secrets', env: envId })}
              >
                Secrets and variables
              </button>
            ) : null
          }
          flush
        >
          <ul className="iii-cloud-cs-needs">
            {fillable.length ? (
              <li className="iii-cloud-cs-filled">
                <MarkIcon mark="bad" label="can be filled" />
                <span>
                  {`${fillable.length} of ${read?.items.length} names your compose file reads can be filled from ${fillFiles.length ? fillFiles.join(' and ') : 'another environment'}. Nothing leaves this machine until you save.`}
                </span>
                <button type="button" className="iii-cloud-linkbtn iii-cloud-small" onClick={() => setEditing(true)}>
                  Review and save
                </button>
              </li>
            ) : null}
            {needItems.map((i) => {
              const name = valueName(i)
              return name ? (
                <SecretField
                  key={i.key}
                  name={name}
                  env={env}
                  why={i.detail ?? i.fix}
                  save={(v) => saveSecret(name, v)}
                />
              ) : (
                <PlainNeed key={i.key} item={i} />
              )
            })}
            {decide.map((o) => (
              <DecisionRow
                key={o.setting}
                o={o}
                env={env}
                file={appFile(s).path}
                canAsk={canAsk}
                pick={(ch) => choose(o, ch)}
              />
            ))}
            {filledText ? (
              <li className="iii-cloud-cs-filled">
                <MarkIcon mark="ok" label="filled" />
                <span>{filledText}</span>
                <button
                  type="button"
                  className="iii-cloud-linkbtn iii-cloud-small"
                  onClick={() => go({ at: 'secrets', env: envId })}
                >
                  Review
                </button>
              </li>
            ) : null}
          </ul>
        </SectionCard>
      ) : null}

      {lines.length || (a && working) ? (
        <SectionCard
          title="Your agent"
          count={lines.length}
          note={
            working && a
              ? `working ${minutesLabel(now - Date.parse(a.started_at))}, ${p?.workspace?.mode === 'checkout' ? 'in your folder' : 'in a separate copy'}`
              : null
          }
          end={
            working && a ? (
              <AgentControls host={host} agent={a} branch={p?.workspace?.branch} />
            ) : lines.length && !running && !stops.some((x) => x.key === 'stopped') ? (
              <Button size="sm" variant="primary" disabled={!canAsk} onClick={ask}>
                {sendLabel(s, 'ready', 'Fix with your agent')}
              </Button>
            ) : null
          }
          flush
        >
          {a && working && (a.state === 'waiting_for_you' || a.state === 'stalled' || folder) ? (
            <p className="iii-cloud-cs-agentnote">
              {a.state === 'waiting_for_you'
                ? (a.question ?? 'Your agent has a question in its chat.')
                : a.state === 'stalled'
                  ? `No progress for ${minutesLabel(idle)}. It may be stuck: open its chat, or stop it.`
                  : folder}
            </p>
          ) : null}
          <ul className="iii-cloud-cs-rows">
            {lines.map((l) => {
              const isNow = l === leader
              return (
                <Fact
                  key={l.key}
                  mark={isNow ? 'work' : 'wait'}
                  title={l.title}
                  sub={isNow ? (a?.note ?? l.detail) : l.detail}
                  end={isNow && a?.current ? <span className="iii-cloud-cs-detail">{`now · ${a.current}`}</span> : null}
                />
              )
            })}
          </ul>
        </SectionCard>
      ) : null}

      {parts ? (
        <>
          <SectionCard title="The check" note={c && !cloudPending(c) ? `commit ${sha7(c.commit)}` : null} flush>
            <div className="iii-cloud-cs-parts">
              {(
                [
                  ["iii cloud's plan", parts.cloud],
                  ['Local run', parts.here],
                ] as const
              ).map(([name, part]) => (
                <section key={name} className="iii-cloud-cs-part" aria-label={name}>
                  <h3>
                    <span>{name}</span>
                    <span className="iii-cloud-cs-state">{part.status}</span>
                  </h3>
                  <Rows rows={part.rows} />
                  {part.foot ? <p className="iii-cloud-cs-foot">{part.foot}</p> : null}
                </section>
              ))}
            </div>
          </SectionCard>
          {running ? (
            <p className="iii-cloud-cs-foot">
              {[
                auto
                  ? 'Your agent starts on what it can fix as soon as the check finds it, in a separate copy of your code.'
                  : null,
                'Deploy unlocks when a check passes. You can leave this page; the console tells you when something needs you.',
              ]
                .filter(Boolean)
                .join(' ')}
            </p>
          ) : null}
        </>
      ) : null}

      {ready && c && !runPending ? <Ready s={s} c={c} host={host} /> : null}

      {c && (doneN || warnings.length || past.length) ? (
        <Details summary={summary}>
          <Group title="Done" count={doneN}>
            {fixed.map((i) => (
              <Fact key={i.key} mark="ok" title={i.title} />
            ))}
            {fromFile.map((o) => (
              <Fact
                key={`file:${o.setting}`}
                mark="ok"
                title={`At deploy, ${o.setting} comes from your compose file.`}
              />
            ))}
            {!needN && filledText ? <Fact mark="ok" title={filledText} /> : null}
          </Group>
          <Group title="Warnings" count={warnings.length}>
            {warnings.map((i) => (
              <Fact
                key={i.key}
                mark="note"
                title={i.title}
                sub={[i.detail, i.fix].filter(Boolean).join(' ')}
                end={
                  configAction(i) ? (
                    <span className="iii-cloud-cs-acts-row">
                      <Button size="sm" variant="secondary" onClick={() => setImporting(true)}>
                        {`Copy ${configDir(i.problem) ?? 'the config folder'} to iii cloud`}
                      </Button>
                      {envPageUrl(s, 'config') ? (
                        <LinkButton href={envPageUrl(s, 'config') as string}>Open in iii cloud</LinkButton>
                      ) : null}
                    </span>
                  ) : null
                }
              />
            ))}
          </Group>
          <Group title="Checks" count={past.length}>
            {past.map((x, i) => (
              <li key={x.id} className="iii-cloud-cs-hist">
                <Mono>{sha7(x.commit)}</Mono>
                <span>{`${i === 0 ? 'Your commits' : 'After a fix'} · ${x.ready ? 'passed' : x.problems ? plural(x.problems, 'problem') : 'no problems'}`}</span>
                <span className="iii-cloud-cs-state">{clock(x.checked_at)}</span>
              </li>
            ))}
          </Group>
        </Details>
      ) : null}

      <ActionBar narrow={narrow} action={action} note={note} />
      {importing ? (
        <ConfigImportDialog
          host={host}
          status={s}
          envId={envId}
          env={env}
          dir={configDir(configItem?.problem) ?? 'the config folder'}
          after="then the check runs again"
          onClose={() => setImporting(false)}
          onApplied={checkAgain}
        />
      ) : null}
      {editing ? (
        <ValuesDialog
          host={host}
          status={s}
          narrow={narrow}
          manifest={c?.manifest || null}
          read={values}
          after="and check again"
          onClose={() => setEditing(false)}
          onSaved={checkAgain}
        />
      ) : null}
      {p?.environment && c ? (
        <TypedConfirm
          open={confirming}
          onOpenChange={setConfirming}
          name={env}
          title={`Deploy ${sha7(c.commit)} of ${p.cloud_project?.name ?? p.repository} to ${env}?`}
          description={
            pr
              ? `This merges pull request #${pr.number} (${pr.head}) into ${pr.base}. The merge starts the deploy in iii cloud, and this page follows it.`
              : `This pushes ${sha7(c.commit)} to ${p.environment.branch} and starts the deploy in iii cloud; this page follows it.`
          }
          confirmLabel={pr ? 'Merge and deploy' : 'Deploy'}
          onConfirm={deploy}
        />
      ) : null}
    </div>
  )
}

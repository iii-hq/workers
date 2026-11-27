// The one screen every deploy from this page goes through, as far as the check: its header says the next thing
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
import { useDeployments } from './api'
import { askedAt, clock, FOLDER_NOTE, useFolderNote, useLaunch } from './asked'
import { ConfigImportDialog } from './ConfigImport'
import { configAction, findings, type Item, location, needsYou } from './findings'
import { canOpenChat, handOff, openAgentChat } from './handoff'
import { hubOf, useActions, useCheckError, useChecking, useLocked, useMovedFrom } from './hub'
import {
  ASSISTED,
  activeSameKind,
  appFile,
  CHECK_AGAIN,
  cloudPending,
  cloudPendingNote,
  configDir,
  conflictFiles,
  deployBusy,
  duration,
  envHead,
  envPageUrl,
  fixBranch,
  isFailedDeploy,
  listMayHideServing,
  minutesLabel,
  needsAction,
  openProblems,
  opensToPublic,
  plainDetail,
  plainMessage,
  plural,
  postgresNote,
  problemTitle,
  RESOURCE_KIND,
  recentDeployRange,
  roleOf,
  sendLabel,
  servingNamed,
  settingWords,
  sha7,
  signInRequired,
  sourceLabel,
  stageLabel,
  stageText,
  stageTime,
  typicalDeployMs,
  usablePr,
  wayOf,
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
import type { AgentState, Check, Deploy, Status, WorkerError } from './types'
import { ValuesCard } from './ValuesCard'
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
  /** "Change" was pressed: its button, so the panel can give the focus back to this very one. */
  onChange?: (trigger: HTMLElement) => void
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
type Act = {
  label: string
  onClick?: () => void
  href?: string
  disabled?: boolean
  /** It deploys: it obeys the same locks as the primary deploy action. */
  deploys?: boolean
}
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
const UNLOCKS = 'Unlocks when a check passes'
const ACTIVE: AgentState[] = ['starting', 'working', 'waiting_for_you', 'waiting', 'stalled']
const MARKS = {
  ok: [Done, 'done'],
  bad: [CircleAlert, 'problem'],
  work: [Loader2, 'working'],
  wait: [CircleDashed, 'next'],
  note: [Info, 'warning'],
} as const

const elapsed = (ms: number) => {
  const t = Math.max(0, Math.round(ms / 1000))
  return t < 60 ? `${t} s` : `${Math.floor(t / 60)} min ${t % 60} s`
}
const agentOn = (s: Status) => !!s.agent && ACTIVE.includes(s.agent.state)
/** The agent is on the check's fixes (a ready or deploy-fix hand-off); stalled stays here. */
const fixing = (s: Status) => agentOn(s) && (s.agent?.kind ?? s.handoff_brief?.kind ?? 'ready') !== 'pull_request'
const github = (s: Status, path: string) => `https://github.com/${s.project?.repository ?? ''}/${path}`

/**
 * Why nothing that sends `iii-cloud::deploy` may be pressed now, or null: an out-of-date page or a sign-in to renew,
 * a deploy being sent, the agent fixing a failed deployment, another deployment going. One answer for every control
 * (the primary action, "without it", Confirm on a held deployment, and the production dialog's own button).
 */
export function deployLockReason(
  s: Status,
  envId: string,
  o: { stale: boolean; sending: boolean; exceptId?: string },
): string | null {
  if (o.stale) return 'The page is out of date or signed out. Sign in or refresh first.'
  if (o.sending) return 'A deploy is being sent.'
  if (agentOn(s) && (s.agent?.kind ?? s.handoff_brief?.kind) === 'deploy_fix')
    return 'Your agent is fixing a failed deployment. Its fix goes through the check first.'
  const other = s.deploys.find(
    (d) => d.environment_id === envId && d.id !== o.exceptId && d.outcome === 'running' && !heldElsewhere(d),
  )
  return other ? `Deployment #${other.number} is still going.` : null
}

/**
 * A deployment iii cloud holds for confirmation that is not this page's: the worker's own markers on the record
 * (a `check_id`, or the flag it sets when it approved or created it) say it started it. A matching commit proves
 * nothing: a push by hand of the checked commit was planned before this check and may differ from its plan, so it
 * is not ours to confirm here. A held one without the markers is "elsewhere": it locks nothing, and deploying from
 * the check replaces it. A held one that is ours is confirmed on its own screen.
 */
export const heldElsewhere = (d: Deploy) =>
  needsAction(d) && !(d.check_id || d.approved_by_worker || d.from_console === false)

/** `check` for this screen: it runs the local run, pushes the fix branch, keeps to a picked compose file. */
export function checkPayload(s: Status, manifest = appFile(s).path): Record<string, unknown> {
  // Always named: a check with none stays on the newest check's file, so going back to the environment's own is an ask too.
  return { run: true, ...(fixBranch(s) ? { push: true } : {}), manifest }
}

/**
 * `check` again: the compose file the person picked with a change that has no newer check yet, else the shown
 * check's own. (After a refused check the shown one is of the replaced source, so its file is not what to retry.)
 */
export function retryPayload(host: Host, s: Status): Record<string, unknown> {
  const picked = hubOf(host).pickedManifest(s.project?.environment?.id ?? '')
  return checkPayload(s, picked ?? undefined)
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

/** "feat/search-rank: 3 commits, to dev": what goes, from what the check and the folder carry. */
function goesLine(s: Status, c: Check | null, env: string): string {
  const p = s.project
  const source = sourceLabel(s, env)
  const list = c?.commits?.length ? c.commits : p?.checkout.not_served
  const commits = list?.length ?? 0
  return `${source}: ${commits ? plural(commits, 'commit') : `nothing ${env} doesn't have`}, to ${env}`
}

/** The line under every title: what goes, the compose file, and "Change". */
export function GoesSub({
  s,
  c,
  env,
  onChange,
  replaced,
  file,
}: {
  s: Status
  c: Check | null
  env: string
  onChange?: (trigger: HTMLElement) => void
  /** The shown check is of a source the person replaced: no old commit count, and the file that was picked. */
  replaced?: boolean
  file?: string
}) {
  return (
    <p className="iii-cloud-cs-sub">
      {replaced ? `${sourceLabel(s, env)}, to ${env}` : goesLine(s, c, env)} · <Mono>{file ?? appFile(s).path}</Mono>
      {onChange ? (
        <>
          {' · '}
          <button
            type="button"
            className="iii-cloud-linkbtn iii-cloud-small"
            onClick={(e) => onChange(e.currentTarget)}
          >
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
        <Button
          size="sm"
          variant="secondary"
          disabled={!status.check || !host.chat?.compose || launching}
          onClick={ask}
        >
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
          <Button size="sm" variant="secondary" disabled={checking} onClick={checkAgain}>
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
  locked,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  name: string
  title: string
  description: string
  confirmLabel: string
  onConfirm: () => Promise<void>
  /** Why it can't be confirmed now (a lock that appeared while the dialog is open); null: it can. */
  locked?: string | null
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
          {locked ? (
            <span className="iii-cloud-cs-sub" role="status">
              {locked}
            </span>
          ) : null}
          <Button variant="primary" disabled={busy || !!locked || typed.trim() !== name} onClick={() => void confirm()}>
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
      // Only a blocking problem is a ✕ that needs someone; a warning is a note, said so in its own row.
      mark: p.severity === 'block' ? 'bad' : 'note',
      label: problemTitle(p),
      state: p.severity !== 'block' ? 'warning' : p.owner === 'person' ? 'needs you' : 'needs your agent',
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
        : // The rows it shows (the person's values are the values card's): the count is theirs.
          rows.some((r) => r.mark === 'bad')
          ? `${rows.filter((r) => r.mark === 'bad').length} to fix${plan.at_least ? ', at least' : ''}`
          : 'waits on values',
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

type Line = { key: string; title: ReactNode; detail: string | null; id: string | null }

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
      {mark === 'work' || mark === 'wait' ? (
        <span className="iii-cloud-ring" data-mark={mark} role="img" aria-label={MARKS[mark][1]} />
      ) : (
        <MarkIcon mark={mark} />
      )}
      <div className="iii-cloud-cs-body">
        <span className="iii-cloud-cs-label">{title}</span>
        {sub ? <span className="iii-cloud-cs-detail">{sub}</span> : null}
        {end}
      </div>
    </li>
  )
}

/* ── Ready: what this deploy does ──────────────────────────────────── */

/** A link to GitHub, as the canvas draws it: text, underlined, no button. */
function TextLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a className="iii-cloud-link iii-cloud-small" href={href} target="_blank" rel="noreferrer">
      {children}
    </a>
  )
}

function Ready({ s, c, host }: { s: Status; c: Check; host: Host }) {
  const p = s.project
  const env = p?.environment
  const launching = useLaunch()?.kind === 'launching'
  const feed = useDeployments(host, s, env?.id ?? '')
  if (!p || !env) return null
  // Production deploys by pull request; every other environment gets an assisted deploy.
  const assisted = !env.production
  const way = wayOf(s, env)
  const target = way.way === 'assisted' && way.own ? way.own : env.branch
  // `taken`: another environment follows the branch this deploy sends to; Deploy refuses it.
  const follower = p.environments.find((e) => e.name !== env.name && e.branch === target)?.name
  const pr = assisted ? null : usablePr(s)
  const file = appFile(s)
  const { resources, overrides } = c.plan.confirmations
  const role = roleOf(s)
  // Named by iii cloud's list (a rollback in the console moves it); until the list is read, none is named.
  const serving = servingNamed(s, env.id, feed.data?.deployments)
  const typical = typicalDeployMs(s, env.id)
  const range = recentDeployRange(s, env.id)
  const branch = p.workspace?.source ?? p.checkout.branch
  // Whose commits (newest first): the agent's down to `from_head`, the checkout's from it. No `from_head`: all the agent's.
  const commits = c.commits ?? []
  const from = p.workspace?.from_head
  const at = from ? commits.findIndex((x) => x.sha === from) : -1
  const agent = !from ? commits.length : at >= 0 ? at : from === c.base ? commits.length : null
  const mine = agent === null ? 0 : commits.length - agent
  const whose =
    agent === null
      ? null
      : [mine > 0 ? `${mine} of yours` : null, agent ? `${plural(agent, 'fix', 'fixes')} by your agent` : null]
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
  return (
    <>
      <SectionCard title="What this deploy does" flush>
        <DetailRow
          label="Code"
          note={whose}
          action={
            <TextLink href={github(s, c.base ? `compare/${c.base}...${c.commit}` : `commit/${c.commit}`)}>
              See changes
            </TextLink>
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
          action={<TextLink href={github(s, `blob/${c.commit}/${file.path}`)}>See the file</TextLink>}
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
        {fixes && branch ? (
          <DetailRow
            label="Your branch"
            note={assisted ? `The next deploy starts from ${target}.` : 'The next deploy starts from it.'}
          >
            {assisted
              ? `After the deploy, ${target} has ${fixes}, and ${branch} is not changed`
              : `When the pull request is merged, ${env.branch} gets ${fixes}`}
          </DetailRow>
        ) : null}
        {assisted ? (
          <>
            <DetailRow
              label="How it ships"
              note={`This page sends ${sha7(c.commit)} to ${target}, and iii cloud deploys it.`}
            >
              {ASSISTED}
            </DetailRow>
            {way.way === 'assisted' && way.taken ? (
              <DetailRow
                label="Branch"
                note={`Deploy will refuse: ${target} cannot be followed by two environments. Change what ${env.name} or ${follower ?? 'the other one'} follows in iii cloud, then check again.`}
              >
                {`${follower ?? 'Another environment'} follows ${target}`}
              </DetailRow>
            ) : null}
            {way.way === 'assisted' && way.own ? (
              <DetailRow
                label="Branch"
                note={`${env.branch} is not pushed.${serving ? ` After this deploy, a merge into ${env.branch} no longer deploys ${env.name}.` : ''}`}
              >
                {`${env.name} moves from ${env.branch} to its own branch, ${way.own}`}
              </DetailRow>
            ) : null}
          </>
        ) : (
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
            : feed.data
              ? listMayHideServing(feed.data)
                ? `${env.name} serves a deployment iii cloud's list does not show; its console names it.`
                : `Nothing is live on ${env.name} yet.`
              : null,
          typical ? `It takes about ${minutesLabel(typical)}${range ? `; the last deploys took ${range}` : ''}.` : null,
          "Only you can confirm this; your agent can't.",
          'A new commit before you confirm needs a new check.',
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
  const { error, busy, call, setError } = useActions(host)
  const folder = useFolderNote(s.check?.id) ? FOLDER_NOTE : undefined
  const launching = useLaunch()?.kind === 'launching'
  const [confirming, setConfirming] = useState(false)
  // The check the production name dialog was opened for: it deploys that one, only while it is still the ready one.
  const [confirmFor, setConfirmFor] = useState<string | null>(null)
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
  const source = sourceLabel(s, env)

  // The agent starts by itself when the check this page saw running ends with agent-owned problems, if auto_fix is
  // on, and unless the person stopped it since (until they check again or ask).
  // ponytail: only while the page is open.
  // A `project::use` that changed the source holds Deploy (and the agent) back until another check exists.
  const movedFrom = useMovedFrom(host, envId)
  useEffect(() => {
    if (running) {
      watch.current ??= { base: fresh ? null : (c?.id ?? null) }
      return
    }
    const w = watch.current
    if (!w || !c || c.id === w.base) return
    watch.current = null
    // Not for the check of the source the person replaced: its problems are of what they left.
    if (movedFrom === c.id) return
    if (!auto || c.ready || c.handoff_id || agentOn(s) || askedAt(c.id) || hubOf(host).stopped(envId)) return
    if (openProblems(s).some((x) => x.owner === 'agent')) void handOff(host, s, 'ready')
  })

  // A refusal of the last `check` call is the hub's (it outlives the card that asked): this screen keeps no second
  // copy. Its own errors are those of the other calls (deploy, save, choose), and go when another check arrives.
  const refused = useCheckError(host, envId)
  const stale = useLocked(host, s)
  const checkId = c?.id
  useEffect(() => {
    if (checkId) setError(null)
  }, [checkId, setError])
  const shownError =
    error && !(refused && error.code === refused.code && error.message === refused.message) ? error : null

  const checkAgain = () => void call('check', 'iii-cloud::check', retryPayload(host, s))
  const ask = () => void handOff(host, s, 'ready')
  const canAsk = !!host.chat?.compose && !launching

  const values = useValues(host, s, c ? { manifest: c.manifest || null } : null)
  // The review editor over these names (the values card's "Set values"); null: closed.
  const [editing, setEditing] = useState<string[] | null>(null)

  if (!c && !running) {
    return (
      <div className="iii-cloud-cs" data-narrow={narrow ? '' : undefined}>
        <ScreenHead
          title={`Check ${source} for ${env}`}
          sub={<GoesSub s={s} c={null} env={env} onChange={onChange} />}
          action={<PageAction onClick={() => undefined} disabled>{`Deploy to ${env}`}</PageAction>}
          narrow={narrow}
        />
        {panel}
        {shownError ? <ActionError host={host} status={s} error={shownError} checkAgain={checkAgain} go={go} /> : null}
        <Notice
          variant="info"
          headline="Not checked yet"
          detail="A check runs iii cloud's plan and the local run on this commit. Nothing is deployed."
          action={
            <Button variant="secondary" onClick={checkAgain}>
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
  // A variable or a secret is never a row of the check: the names the deploy cannot go without are one row ("N values
  // need you"), opened in the values review. Optional ones are listed on Secrets and variables, where they are set.
  const read = values.data
  const { items: needItems, decide, values: missing, count: needN } = needsYou(s, c?.ready ? null : read)
  const decisions = c ? c.plan.confirmations.overrides.filter((o) => chosen[o.setting] !== 'file') : []
  const fromFile = c ? c.plan.confirmations.overrides.filter((o) => chosen[o.setting] === 'file') : []
  const agentItems = f.agent.filter((i) => !i.fixed && i.severity === 'block')
  const lines: Line[] = [
    ...agentItems.map((i) => ({
      key: i.key,
      title: i.title,
      detail: i.detail ?? i.fix,
      id: i.problem?.id ?? null,
    })),
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
  // The source was replaced (`project::use`) after this check ran: it says nothing about the new source.
  // (By id, not by time: the worker's clock is its own.)
  const staleSource = !!c && movedFrom === c.id
  // The check is of a source the person replaced: none of its lists, parts or numbers is the current state, so they
  // are folded under Details (closed) and the screen asks for a new check.
  const replaced = staleSource && !running
  const ready = !!c?.ready && !running && !deployFix && !staleSource
  const working = fixing(s) && !ready
  const a = s.agent

  const assisted = !!p?.environment && !p.environment.production
  const pr = assisted ? null : usablePr(s)
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
  const startDeploy = () => {
    if (!production) return void deploy()
    setConfirmFor(c?.id ?? null)
    setConfirming(true)
  }

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
    if (staleSource && !running) {
      stops.push({
        key: 'source',
        variant: 'info',
        headline: 'This check is of the source you replaced',
        detail: refused
          ? "The check of the new source didn't start; its answer is above."
          : `${sha7(c.commit)} was checked before you changed what goes to ${env}. Only a check of the new source can be deployed.`,
        acts: [{ label: 'Check again', onClick: checkAgain }],
        lock: 'Check again first.',
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
      const useCheckout = async () => {
        const used = await call(
          'use',
          'iii-cloud::project::use',
          {
            path: p?.path,
            environment_id: envId,
            mode: ws?.mode,
            from: 'checkout',
            ...(ws?.source ? { source: ws.source } : {}),
          },
          { changesSource: true },
        )
        if (used !== null) checkAgain()
      }
      stops.push({
        key: 'outdated',
        variant: 'info',
        headline: `${source} moved after the check`,
        detail: `${sha7(c.commit)} ${c.ready ? 'passed' : 'was checked'}, but ${source} now ends at ${sha7(tipHere)}. Only a commit that passed can be deployed.`,
        acts: [
          { label: `Check ${sha7(tipHere)}`, onClick: () => void useCheckout(), disabled: running },
          ...(c.ready ? [{ label: `Deploy ${sha7(c.commit)} without it`, onClick: startDeploy, deploys: true }] : []),
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
    if (d && heldElsewhere(d)) {
      // Waiting for confirmation in iii cloud, started elsewhere: nothing here can confirm it, and deploying replaces it.
      stops.push({
        key: 'held',
        variant: 'info',
        headline: `Deployment #${d.number} is waiting for confirmation in iii cloud`,
        detail: `Deploying ${env} replaces it.`,
        acts: [],
      })
      // A deployment going locks Deploy whatever check it came from: its progress is the screen for that state.
    } else if (d?.outcome === 'running') {
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
    !assisted && ready && (!pr || pr.main_moved)
      ? 'Open the pull request, and check again if the branch moved.'
      : undefined
  const staleDeploy = !!error && CHECK_AGAIN.has(error.code)
  // Every control that deploys obeys these: another deployment going, signed out, an out-of-date page, a deploy being
  // sent, the agent fixing a failed deployment. (Its own stop's lock, "Check X first", is the one it overrides.)
  const deployLock =
    stops.find((x) => x.lock && x.key !== 'outdated')?.lock ??
    (staleDeploy ? 'Check again first.' : null) ??
    deployLockReason(s, envId, { stale, sending: busy === 'deploy' })
  const deployLocked = !!deployLock
  // The local run never ran on a commit iii cloud's plan accepts (a failed run has its own stop).
  const runPending = !!c && !running && c.plan.status === 'clean' && !run && !agentItems.length && !needN && !c.ready

  const youN = needItems.length + decide.length
  const secretsOnly = missing.length > 0 && !youN
  // The header: the next thing to do, said with the lists' own lengths.
  const title = replaced
    ? `Check ${source} for ${env}`
    : running && !working
      ? `Checking ${source} for ${env}`
      : ready
        ? `Ready to deploy to ${env}`
        : runPending
          ? `iii cloud's plan accepts ${sha7(c?.commit ?? '')}; the local run is left`
          : needN || lines.length || working
            ? [
                needN
                  ? secretsOnly
                    ? `${plural(missing.length, 'value')} to set`
                    : `${plural(needN, 'thing')} ${needN === 1 ? 'needs' : 'need'} you`
                  : null,
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
  // The checks of the agent's hand-off, from the first one's start to the last one's end.
  const spent =
    past.length > 1
      ? Date.parse(past[past.length - 1].checked_at) - (Date.parse(past[0].checked_at) - past[0].duration_ms)
      : 0
  const sub = ready ? (
    <p className="iii-cloud-cs-sub">
      <Mono>{c ? sha7(c.commit) : ''}</Mono>
      {` passed iii cloud's plan${run ? ' and the local run' : ''} at ${clock(ranAt ?? '')}${past.length > 1 ? ` · ${past.length} checks${spent > 0 ? `, ${minutesLabel(spent)}` : ''}` : ''}`}
      {onChange ? (
        <>
          {' · '}
          <button
            type="button"
            className="iii-cloud-linkbtn iii-cloud-small"
            onClick={(e) => onChange(e.currentTarget)}
          >
            Change
          </button>
        </>
      ) : null}
    </p>
  ) : (
    <GoesSub
      s={s}
      c={c}
      env={env}
      onChange={onChange}
      replaced={replaced}
      file={replaced ? (hubOf(host).pickedManifest(envId) ?? undefined) : undefined}
    />
  )

  // On a replaced source nothing of the old check offers work: only the source notice and the sign-in stay.
  const shownStops = replaced ? stops.filter((x) => x.key === 'source' || x.key === 'signin') : stops
  // `note` is the reason under the header's action (only a lock says one: the canvas draws no line otherwise);
  // `barNote` is the phone bar's line under the same action.
  let action: ReactNode
  let note: ReactNode = null
  let barNote: ReactNode = null
  if (ready || runPending) {
    // The same lock reason as every other control that sends `iii-cloud::deploy` (`deployLock`), plus this action's own.
    const off = ready ? (lock ?? prLock ?? deployLock) : undefined
    action = ready ? (
      <PageAction disabled={!!off} onClick={startDeploy}>
        {assisted
          ? `Deploy ${c ? sha7(c.commit) : ''} to ${env}${production ? '…' : ''}`
          : `Merge to ${pr?.base ?? p?.environment?.branch} and deploy to ${env}${production ? '…' : ''}`}
      </PageAction>
    ) : (
      <PageAction disabled={running} onClick={checkAgain}>
        Start the local run
      </PageAction>
    )
    note = off ?? null
    barNote = off ?? (ready ? "Only you can confirm this; your agent can't." : null)
  } else {
    // Not ready and no check in flight: checking again is always on screen. With nothing else for the person to do
    // (the agent has nothing, no list for them) it is the primary action; otherwise it sits beside the locked Deploy.
    const idle = !running && !hubChecking
    const nothingElse = replaced || (!needN && !lines.length && !working)
    const deployOff = <PageAction onClick={() => undefined} disabled>{`Deploy to ${env}`}</PageAction>
    // A panel below that checks again is the one place for it: the header does not say it twice.
    const panelChecks = shownStops.some((x) => x.key === 'outdated' || x.acts.some((a) => a.onClick === checkAgain))
    action = panelChecks ? (
      deployOff
    ) : idle && nothingElse ? (
      <PageAction onClick={checkAgain}>Check again</PageAction>
    ) : idle ? (
      <span className="iii-cloud-row">
        <Button variant="secondary" onClick={checkAgain}>
          Check again
        </Button>
        {deployOff}
      </span>
    ) : (
      deployOff
    )
    barNote = UNLOCKS
  }

  const parts =
    !replaced && !ready && (running || (!needN && !lines.length))
      ? {
          cloud: c && fresh ? cloudPart(s, c) : skeleton('checking', 4),
          here: c && fresh ? runPart(s, c, now, running) : skeleton('next', 5),
        }
      : null
  const pending = c && !running && !ready && !replaced ? cloudPendingNote(s) : null
  // " · 14 s so far, about 20 s left": while it runs; the time left is the median of this environment's earlier checks.
  const took = (s.checks ?? [])
    .filter((x) => x.environment_id === envId && x.id !== c?.id && x.duration_ms > 0)
    .map((x) => x.duration_ms)
    .sort((x, y) => x - y)
  const spentNow = running && run?.status === 'running' ? now - Date.parse(run.started_at) : null
  const left = spentNow != null && took.length ? took[Math.floor(took.length / 2)] - spentNow : 0
  const progress =
    spentNow != null ? ` · ${elapsed(spentNow)} so far${left >= 1000 ? `, about ${elapsed(left)} left` : ''}` : ''
  const youNote = running ? 'You can do this while the check runs' : "Your agent can't do these"

  // "Skip for now" and its Undo: the workers' config warning (`problem::skip`); then the check runs again.
  const skipConfig = async (on: boolean) => {
    const out = await call('skip', 'iii-cloud::problem::skip', {
      environment_id: envId,
      code: 'LOCAL_CONFIG_NOT_SET',
      skip: on,
    })
    if (out !== null) checkAgain()
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
  const doneN = fixed.length + fromFile.length
  // Passed: who did what is the news ("You did 2 · your agent fixed 2"); before that, one count of what is done.
  const agentFixed = fixed.filter((i) => i.owner === 'agent').length
  const summary = [
    ready ? `You did ${doneN - agentFixed} · your agent fixed ${agentFixed}` : `Done ${doneN}`,
    `Warnings ${warnings.length}`,
    `Checks ${past.length}${lastCheck && !ready ? ` (last ${clock(lastCheck.checked_at)}, ${duration(lastCheck.duration_ms)})` : ''}`,
  ].join(' · ')

  return (
    <div className="iii-cloud-cs" data-narrow={narrow ? '' : undefined}>
      <ScreenHead title={title} chip={chip} sub={sub} action={action} note={note} narrow={narrow} />
      {panel}
      {pending ? <Notice variant="info" headline="iii cloud's plan is pending" detail={pending} /> : null}
      {shownError ? <ActionError host={host} status={s} error={shownError} checkAgain={checkAgain} go={go} /> : null}
      {shownStops.map((x) => (
        <Notice
          key={x.key}
          className="iii-cloud-cs-stop"
          variant={x.variant}
          headline={x.headline}
          detail={x.detail}
          action={
            <span className="iii-cloud-cs-acts">
              {x.acts.map((act) =>
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
                    variant="secondary"
                    disabled={act.disabled || (act.deploys && deployLocked)}
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

      {youN && !replaced ? (
        <SectionCard title="You" count={youN} note={youNote} flush>
          <ul className="iii-cloud-cs-needs">
            {needItems.map((i) => (
              <PlainNeed key={i.key} item={i} />
            ))}
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
          </ul>
        </SectionCard>
      ) : null}

      {!replaced ? (
        <ValuesCard
          read={values}
          onSet={(names) => setEditing(names)}
          onSkip={async (names, skip) => {
            const out = await call('skip', 'iii-cloud::values::skip', { environment_id: envId, names, skip })
            if (out !== null) values.refetch()
          }}
          onAll={() => go({ at: 'secrets', env: envId })}
        />
      ) : null}

      {!replaced && (lines.length || (a && working)) ? (
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
              <Button size="sm" variant="secondary" disabled={!canAsk} onClick={ask}>
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
                  ? (a.question ?? `No progress for ${minutesLabel(idle)}. It may be stuck: open its chat, or stop it.`)
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
        <SectionCard
          title="The check"
          note={c && !cloudPending(c) ? `commit ${sha7(c.commit)}${progress}` : null}
          flush
        >
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
      ) : null}

      {ready && c && !runPending ? <Ready s={s} c={c} host={host} /> : null}

      {c && (replaced || doneN || warnings.length || past.length) ? (
        <Details summary={replaced ? `Check of the replaced source · ${summary}` : summary}>
          {replaced ? (
            <Group title="The check before you changed the source" count={1}>
              <li className="iii-cloud-cs-need">
                <span className="iii-cloud-cs-detail">
                  {`${sha7(c.commit)}: ${needN ? `${plural(needN, 'thing')} for you, ` : ''}${lines.length ? `${plural(lines.length, 'thing')} for your agent, ` : ''}${plural(warnings.length, 'warning')}. It says nothing about the new source.`}
                </span>
              </li>
            </Group>
          ) : null}
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
          </Group>
          <Group title="Warnings" count={warnings.length}>
            {warnings.map((i) => (
              <Fact
                key={i.key}
                mark="note"
                title={i.title}
                sub={[i.detail, i.fix].filter(Boolean).join(' ')}
                end={
                  // On a replaced source no action of the old check is offered (skip, undo, copy the config).
                  replaced ? null : i.skipped ? (
                    <span className="iii-cloud-cs-acts-row">
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={busy === 'skip'}
                        onClick={() => void skipConfig(false)}
                      >
                        Undo
                      </Button>
                    </span>
                  ) : configAction(i) ? (
                    <span className="iii-cloud-cs-acts-row">
                      <Button size="sm" variant="secondary" onClick={() => setImporting(true)}>
                        {`Copy ${configDir(i.problem) ?? 'the config folder'} to iii cloud`}
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={busy === 'skip'}
                        onClick={() => void skipConfig(true)}
                      >
                        Skip for now
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

      {!ready && !replaced && (running || needN > 0 || lines.length > 0) ? (
        <p className="iii-cloud-cs-foot">
          {[
            running && auto
              ? 'Your agent starts on what it can fix as soon as the check finds it, in a separate copy of your code.'
              : null,
            'Deploy unlocks when a check passes.',
            !running && (working || lines.length > 0) ? 'Every fix by your agent runs the check again.' : null,
            'You can leave this page; the console tells you when something needs you.',
          ]
            .filter(Boolean)
            .join(' ')}
        </p>
      ) : null}

      <ActionBar narrow={narrow} action={action} note={barNote} />
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
          only={editing}
          describe="Set them from a file, type them, leave them empty on purpose, or skip the optional ones: iii cloud does not deploy without the required ones."
          host={host}
          status={s}
          narrow={narrow}
          manifest={c?.manifest || null}
          read={values}
          after="and check again"
          onClose={() => setEditing(null)}
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
          locked={
            ready && confirmFor !== c.id
              ? 'The ready check changed while this was open. Close it and deploy the new one.'
              : !ready
                ? 'This is no longer the check that was ready to deploy. Check again.'
                : (lock ?? prLock ?? deployLockReason(s, envId, { stale, sending: false }))
          }
        />
      ) : null}
    </div>
  )
}

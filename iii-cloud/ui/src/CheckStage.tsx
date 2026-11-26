import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  Checkbox,
  Chip,
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
  EmptyState,
  Skeleton,
} from '@iii-dev/console-ui'
import { usePaneState } from '@iii-dev/console-ui/hooks'
import { ChevronDown, ChevronRight, CircleAlert, CircleDashed, Check as Done, Info, Loader2 } from 'lucide-react'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { AgentRow } from './AgentRow'
import { askedAt, clock, FOLDER_NOTE, useAsked, useFolderNote, useLaunch } from './asked'
import { ConfigImportDialog } from './ConfigImport'
import {
  codeLines,
  configAction,
  findings,
  type Group,
  groupItems,
  groupLocation,
  type Item,
  location,
  personAction,
  sharedDetail,
  titleOf,
  valueName,
} from './findings'
import { canOpenChat, handOff, openAgentChat } from './handoff'
import { useActions, useChecking } from './hub'
import {
  appFile,
  cloudPending,
  cloudPendingNote,
  configDir,
  conflictFiles,
  deployBusy,
  duration,
  envHead,
  envPageUrl,
  fixBranch,
  isDirty,
  openProblems,
  opensToPublic,
  optionalUnset,
  plainMessage,
  plural,
  postgresNote,
  problemTitle,
  sendLabel,
  settingWords,
  sha7,
  signInRequired,
  stageLabel,
  stageText,
  stageTime,
  withAppFile,
  workingOnIds,
} from './model'
import { ErrorPanel, LinkButton, Mono, Notice, StatusChip, StepHeader, useNow } from './parts'
import type { Route } from './route'
import type { AgentState, Check, OverrideChoice, Status, WorkerError } from './types'
import { ValuesDialog } from './ValuesEditor'

export type CheckStageProps = {
  host: Host
  status: Status
  /** `setup`: step 4 of setting up a project; `change`: the check before deploying from this machine. */
  scenario: 'setup' | 'change'
  go: (r: Route) => void
  narrow: boolean
  /** An earlier check, read-only (from the Checks tab), whose record the caller puts in `status.check`. */
  checkId?: string
}

/**
 * The check before every deploy (CheckStage board): checking, problems, your agent fixing, passed.
 * With `checkId` it shows `status.check` as given (the caller read that record) and acts on nothing.
 */
export function CheckStage(props: CheckStageProps) {
  return <Stage {...props} readOnly={!!props.checkId} />
}

type Phase = 'checking' | 'problems' | 'fixing' | 'passed'
type Mark = 'ok' | 'bad' | 'work' | 'wait' | 'note'
type Row = {
  key: string
  mark: Mark
  aria?: string
  label: ReactNode
  state: string
  part?: string
  detail?: string | null
  log?: { title: string; lines: string[] } | null
}
type Act = { label: string; onClick?: () => void; href?: string; disabled?: boolean }
/** A Dep7 panel: why the check can't finish (or what only the cloud saw), in place of its result. */
type Stop = {
  key: string
  variant: 'info' | 'warn' | 'alert'
  headline: string
  detail: ReactNode
  acts: Act[]
  /** Its first action is the view's primary one (false: the panel only informs). */
  lead?: boolean
  /** It keeps "Review and deploy" locked; the note says why. */
  lock?: string
}

const DOCKER_DOCS = 'https://docs.docker.com/get-started/get-docker/'
const AT_LEAST =
  'iii cloud stops at the first thing it cannot accept, so more can show up after the first round is fixed.'
const UNLOCKS = 'Unlocks when a check passes on the commit you deploy.'
const PART = { cloud_plan: 'iii cloud', check_here: 'here' } as const
const ACTIVE: AgentState[] = ['starting', 'working', 'waiting_for_you', 'waiting', 'stalled']
const MARKS = {
  ok: [Done, 'passed'],
  bad: [CircleAlert, 'problem'],
  work: [Loader2, 'running'],
  wait: [CircleDashed, 'waiting'],
  note: [Info, 'worth a look'],
} as const

const elapsed = (ms: number) => {
  const t = Math.max(0, Math.round(ms / 1000))
  return t < 60 ? `${t} s` : `${Math.floor(t / 60)} min ${t % 60} s`
}
const agentOn = (s: Status) => !!s.agent && ACTIVE.includes(s.agent.state)
/** The agent is on the check's fixes (a ready or deploy-fix hand-off); stalled stays here, AgentRow warns. */
function fixing(s: Status): boolean {
  const kind = s.agent?.kind ?? s.handoff_brief?.kind ?? 'ready'
  return agentOn(s) && (kind === 'ready' || kind === 'deploy_fix')
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

function Rows({ rows }: { rows: Row[] }) {
  return (
    <ul className="iii-cloud-cs-rows">
      {rows.map((r) => (
        <li key={r.key} className="iii-cloud-cs-row" data-mark={r.mark}>
          <MarkIcon mark={r.mark} label={r.aria} />
          <span className="iii-cloud-cs-label">{r.label}</span>
          <span className="iii-cloud-cs-part">{r.part}</span>
          <span className="iii-cloud-cs-state">{r.state}</span>
          {r.detail ? <span className="iii-cloud-cs-detail">{r.detail}</span> : null}
          {r.log?.lines.length ? (
            <details className="iii-cloud-cs-log">
              <summary>{r.log.title}</summary>
              <pre>{r.log.lines.join('\n')}</pre>
            </details>
          ) : null}
        </li>
      ))}
    </ul>
  )
}

/** The person's open/closed choices, by key; a key never toggled follows its default (which moves with the check). */
type Folds = { isOpen: (key: string, byDefault: boolean) => boolean; set: (key: string, open: boolean) => void }
function useFolds(): Folds {
  const [chosen, setChosen] = useState<Record<string, boolean>>({})
  return {
    isOpen: (key, byDefault) => chosen[key] ?? byDefault,
    set: (key, open) => setChosen((c) => ({ ...c, [key]: open })),
  }
}

/** A collapsible panel: the header (chevron, title, count chip, note) is the trigger. */
function Section({
  id,
  folds,
  byDefault,
  title,
  count,
  note,
  foot,
  children,
}: {
  id: string
  folds: Folds
  byDefault: boolean
  title: string
  count?: number
  note?: ReactNode
  foot?: ReactNode
  children: ReactNode
}) {
  const open = folds.isOpen(id, byDefault)
  return (
    <CollapsibleCard
      open={open}
      onOpenChange={(o) => folds.set(id, o)}
      role="region"
      aria-label={title}
      className="iii-cloud-cs-panel"
    >
      <CollapsibleCardTrigger className="iii-cloud-cs-head" aria-expanded={open}>
        <ChevronRight className="iii-cloud-cs-chev" aria-hidden />
        <span className="iii-cloud-cs-title">{title}</span>
        {count != null ? <Chip className="iii-cloud-cs-count">{count}</Chip> : null}
        {note ? <span className="iii-cloud-panel-end iii-cloud-cs-note">{note}</span> : null}
      </CollapsibleCardTrigger>
      <CollapsibleCardContent>
        {children}
        {foot ? <p className="iii-cloud-cs-foot">{foot}</p> : null}
      </CollapsibleCardContent>
    </CollapsibleCard>
  )
}

/** What a section's rows need besides themselves. */
type ListCtx = {
  host: Host
  status: Status
  folds: Folds
  /** Problem ids the agent is on now. */
  working: Set<string>
  /** The run's last lines, and the one problem they go with (the first found here with no excerpt). */
  log: string[]
  logKey: string | undefined
  /** "Set values": the values dialog, all of the file's names or only these (null: read-only). */
  values: ((only?: string[]) => void) | null
  /** O2: the workers' config row's ways out: copy it (the import dialog), skip it for now (or undo), the console's
      config page (null: read-only). */
  config: { copy: () => void; skip: (on: boolean) => void; url: string | null } | null
  /** The environment's name. */
  env: string
  /** Each row's "here" / "iii cloud" tag (off in "Needs you"). */
  parts: boolean
  /** "Skip for now" (`values::skip`), or its undo; then the check runs again (null: read-only). */
  skip: ((names: string[], skip: boolean) => void) | null
  /** A skipped SECRET_MISSING: "Leave it empty in iii cloud" (`values::save` leave_empty), then the check runs again. */
  leaveEmpty: ((names: string[]) => void) | null
  /** The "Skipped" section: its value rows offer Undo. */
  undo?: boolean
}

/** The names a value row is about, once each (its open items only). */
const namesOf = (items: Item[]) => [...new Set(items.filter((i) => !i.fixed).flatMap((i) => valueName(i) ?? []))]

const PART_OF = (i: Item) => (i.problem ? PART[i.problem.source] : undefined)
function markOf(i: Item, working: Set<string>): { mark: Mark; aria: string } {
  if (i.fixed) return { mark: 'ok', aria: 'fixed' }
  if (i.owner === 'none') return { mark: 'note', aria: 'worth a look' }
  if (i.skipped && i.severity !== 'block') return { mark: 'note', aria: 'skipped' }
  if (i.problem && working.has(i.problem.id)) return { mark: 'work', aria: 'Your agent is working on this' }
  return { mark: 'bad', aria: 'problem' }
}

/** One problem: its mark, its title and where it ran; under it the detail, one location line, the code or the log, its action. */
function ItemRow({ item: i, ctx, log }: { item: Item; ctx: ListCtx; log: boolean }) {
  const m = markOf(i, ctx.working)
  const value = !i.fixed && i.owner === 'person' && personAction(i)
  const config = !i.fixed && i.owner === 'person' && configAction(i)
  const dir = configDir(i.problem) ?? 'the config folder'
  // A row the person acts on keeps its words, its location and its action: the code lines would only repeat them.
  const code = i.fixed || value || config ? [] : codeLines(i.problem)
  const detail = config
    ? `Your workers read config from ${dir}, which isn't in the repository. In iii cloud they start with their defaults.`
    : i.detail
  const x = i.problem?.excerpt
  // A bare file the detail already names is not said twice.
  const loc =
    i.line == null && !i.target && i.file && i.detail?.includes(i.file) ? null : location(i.file, i.line, i.target)
  return (
    // O1: a line of the page's own, not the check's (an override the person chose), reads faint.
    <li className="iii-cloud-cs-item" data-mark={m.mark} data-faint={!i.problem && i.owner === 'none' ? '' : undefined}>
      <MarkIcon mark={m.mark} label={m.aria} />
      <div className="iii-cloud-cs-body">
        <span className="iii-cloud-cs-tline">
          <span className="iii-cloud-cs-label">
            {i.title}
            {value && i.name && !i.title.includes(i.name) ? <span className="iii-cloud-cs-at">{i.name}</span> : null}
          </span>
          <span className="iii-cloud-cs-part">{ctx.parts ? PART_OF(i) : null}</span>
        </span>
        {detail && !i.fixed ? <span className="iii-cloud-cs-detail">{detail}</span> : null}
        {loc && !i.fixed ? <span className="iii-cloud-cs-loc">{loc}</span> : null}
        {code.length && x ? (
          <details className="iii-cloud-cs-log">
            <summary>{`Lines ${x.start_line}–${x.start_line + x.lines.length - 1}`}</summary>
            <pre>{code.join('\n')}</pre>
          </details>
        ) : log ? (
          <details className="iii-cloud-cs-log">
            <summary>Last lines of the run here</summary>
            <pre>{ctx.log.join('\n')}</pre>
          </details>
        ) : null}
        {(value && ctx.values) || (config && ctx.config) ? (
          <span className="iii-cloud-cs-acts-row">
            {value ? <ValueActs items={[i]} ctx={ctx} /> : null}
            {config ? <ConfigActs ctx={ctx} dir={dir} /> : null}
          </span>
        ) : null}
      </div>
    </li>
  )
}

/**
 * Problems of one code in one file: the plural sentence, the names as chips, then "Show each": one line per item.
 * Names that default to empty (`${X:-}`) are said neutral: optional, nothing breaks without a value.
 */
function GroupRow({ group: g, ctx }: { group: Group; ctx: ListCtx }) {
  const key = `group:${g.key}`
  const open = ctx.folds.isOpen(key, false)
  const n = g.items.length
  const fixed = g.items.filter((i) => i.fixed).length
  const first = g.items[0]
  const optional = g.code === 'EMPTY_DEFAULT_VARIABLE' && fixed < n
  const marks = g.items.map((i) => markOf(i, ctx.working))
  const m: { mark: Mark; aria: string } = optional
    ? { mark: 'note', aria: 'optional' }
    : fixed === n
      ? marks[0]
      : (marks.find((x) => x.mark === 'work') ?? marks.find((x) => x.mark !== 'ok') ?? marks[0])
  const shared = optional ? null : sharedDetail(g)
  const value = first.owner === 'person' && fixed < n && g.items.some((i) => !i.fixed && personAction(i))
  const loc = groupLocation(g)
  // "Show each": a value group skips (or, under "Skipped", brings back) one name at a time.
  const each = value && ctx.skip && (ctx.undo || !g.items.some((i) => i.skipped)) ? (ctx.undo ? 'Undo' : 'Skip') : null
  return (
    <li className="iii-cloud-cs-item iii-cloud-cs-group" data-mark={m.mark} data-optional={optional ? '' : undefined}>
      <MarkIcon mark={m.mark} label={m.aria} />
      <div className="iii-cloud-cs-body">
        <span className="iii-cloud-cs-tline">
          <span className="iii-cloud-cs-label">
            {optional ? optionalUnset(n - fixed, ctx.env) : titleOf(g)}
            {fixed ? (
              <span className="iii-cloud-cs-gstate">{fixed === n ? 'all fixed' : `${fixed} of ${n} fixed`}</span>
            ) : null}
          </span>
          <span className="iii-cloud-cs-part">{ctx.parts ? PART_OF(first) : null}</span>
        </span>
        <span className="iii-cloud-cs-chips">
          {g.items.map((i) => (
            <Chip key={i.key} data-fixed={i.fixed ? '' : undefined}>
              <Mono>{i.name ?? i.title}</Mono>
            </Chip>
          ))}
        </span>
        {shared ? <span className="iii-cloud-cs-detail">{shared}</span> : null}
        {loc ? <span className="iii-cloud-cs-loc">{loc}</span> : null}
        {open ? (
          <ul className="iii-cloud-cs-each" aria-label="Each one">
            {g.items.map((i) => (
              <li key={i.key} data-fixed={i.fixed ? '' : undefined}>
                {i.fixed ? <MarkIcon mark="ok" label="fixed" /> : null}
                <span>
                  <b className="iii-cloud-cs-each-name">
                    <Mono>{i.name ?? i.title}</Mono>
                  </b>
                  {location(i.file, i.line, null) ? (
                    <span className="iii-cloud-cs-at">{` · ${location(i.file, i.line, null)}`}</span>
                  ) : null}
                </span>
                {each && !i.fixed && valueName(i) ? (
                  <button
                    type="button"
                    className="iii-cloud-linkbtn iii-cloud-small iii-cloud-cs-each-act"
                    aria-label={`${each === 'Undo' ? 'Undo skipping' : 'Skip'} ${valueName(i)}`}
                    onClick={() => ctx.skip?.([valueName(i) ?? ''], each === 'Skip')}
                  >
                    {each}
                  </button>
                ) : null}
                {i.detail && i.detail !== shared ? <span className="iii-cloud-cs-detail">{i.detail}</span> : null}
              </li>
            ))}
          </ul>
        ) : null}
        <span className="iii-cloud-cs-acts-row">
          {value ? <ValueActs items={g.items} ctx={ctx} quiet={optional} /> : null}
          <Button
            size="sm"
            variant="ghost"
            className="iii-cloud-cs-each-toggle"
            aria-expanded={open}
            onClick={() => ctx.folds.set(key, !open)}
          >
            {open ? 'Hide each' : 'Show each'}
            <ChevronDown className="iii-cloud-cs-chev" aria-hidden />
          </Button>
        </span>
      </div>
    </li>
  )
}

/** "Set values", the person's next action (secondary when only optional names wait): every value the file reads, in one dialog (never the agent's). */
function SetValues({ ctx, quiet = false }: { ctx: ListCtx; quiet?: boolean }) {
  return ctx.values ? (
    <Button size="sm" variant={quiet ? 'secondary' : 'primary'} onClick={() => ctx.values?.()}>
      Set values
    </Button>
  ) : null
}

/**
 * A value row's actions: Set values and a ghost "Skip for now". A skipped SECRET_MISSING (iii cloud still needs a
 * value): "Leave it empty in iii cloud" first. Under "Skipped": Undo.
 */
function ValueActs({ items, ctx, quiet = false }: { items: Item[]; ctx: ListCtx; quiet?: boolean }) {
  const names = namesOf(items)
  if (ctx.undo)
    return ctx.skip && names.length ? (
      <Button size="sm" variant="ghost" onClick={() => ctx.skip?.(names, false)}>
        Undo
      </Button>
    ) : null
  const empty = items.some((i) => i.skipped)
  return (
    <>
      {empty && ctx.leaveEmpty ? (
        <Button size="sm" variant="primary" onClick={() => ctx.leaveEmpty?.(names)}>
          Leave it empty in iii cloud
        </Button>
      ) : null}
      <SetValues ctx={ctx} quiet={quiet || empty} />
      {!empty && ctx.skip && names.length ? (
        <Button size="sm" variant="ghost" onClick={() => ctx.skip?.(names, true)}>
          Skip for now
        </Button>
      ) : null}
    </>
  )
}

/** P3: the run here's missing values as one line, never a row (or a chip) per name; Set values opens only them. */
function HereValuesRow({ names, ctx }: { names: string[]; ctx: ListCtx }) {
  const n = names.length
  return (
    <li className="iii-cloud-cs-item" data-mark="bad">
      <MarkIcon mark="bad" label="problem" />
      <div className="iii-cloud-cs-body">
        <span className="iii-cloud-cs-tline">
          <span className="iii-cloud-cs-label">
            {`${plural(n, 'value')} the run here needs ${n === 1 ? "isn't" : "aren't"} on this machine`}
          </span>
        </span>
        {ctx.values ? (
          <span className="iii-cloud-cs-acts-row">
            <Button size="sm" variant="primary" onClick={() => ctx.values?.(names)}>
              Set values
            </Button>
            {ctx.skip ? (
              <Button size="sm" variant="ghost" onClick={() => ctx.skip?.(names, true)}>
                Skip for now
              </Button>
            ) : null}
          </span>
        ) : null}
      </div>
    </li>
  )
}

/** O2: the workers' config row: copy the folder to iii cloud, open the console's config page, skip it; Undo under "Skipped". */
function ConfigActs({ ctx, dir }: { ctx: ListCtx; dir: string }) {
  const k = ctx.config
  if (!k) return null
  if (ctx.undo)
    return (
      <Button size="sm" variant="ghost" onClick={() => k.skip(false)}>
        Undo
      </Button>
    )
  return (
    <>
      <Button size="sm" variant="primary" onClick={k.copy}>
        {`Copy ${dir} to iii cloud`}
      </Button>
      {k.url ? <LinkButton href={k.url}>Open in iii cloud</LinkButton> : null}
      <Button size="sm" variant="ghost" onClick={() => k.skip(true)}>
        Skip for now
      </Button>
    </>
  )
}

function ItemList({ items, ctx, after }: { items: Item[]; ctx: ListCtx; after?: ReactNode }) {
  return (
    <ul className="iii-cloud-cs-rows">
      {groupItems(items).map((g) =>
        g.items.length > 1 ? (
          <GroupRow key={g.key} group={g} ctx={ctx} />
        ) : (
          <ItemRow key={g.key} item={g.items[0]} ctx={ctx} log={g.items[0].key === ctx.logKey} />
        ),
      )}
      {after}
    </ul>
  )
}

/**
 * D4: blocking (and fixed, and the values only the person gives) first; open warnings and the notes are worth a
 * look (O2: the workers' config is a warning too).
 */
const splitWorth = (items: Item[]) => {
  const mine = (i: Item) => i.owner === 'person' && personAction(i)
  const main = items.filter((i) => i.fixed || i.severity === 'block' || mine(i))
  // Open before fixed; blocking, then values (a stable sort keeps the check's order within).
  const rank = (i: Item) => (i.fixed ? 4 : i.severity === 'block' ? 0 : 1)
  return { main: [...main].sort((a, b) => rank(a) - rank(b)), worth: items.filter((i) => !main.includes(i)) }
}

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

/** "iii cloud accepts it": a row per cloud problem; once the plan is clean, what it runs and the console settings. */
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
      sum
        ? {
            key: 'summary',
            mark: 'ok',
            label: 'What the plan runs',
            state: `${plural(sum.workers, 'worker')} in ${plural(sum.groups.length, 'group')}`,
          }
        : { key: 'summary', mark: 'ok', label: `iii cloud accepts ${sha7(c.commit)}`, state: 'clean' },
      {
        key: 'overrides',
        mark: n ? 'bad' : 'ok',
        label: 'Settings saved in the console match your app file',
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

/** "It runs here like the cloud": the run's seven stages, live through the run events. */
function runPart(s: Status, c: Check, now: number, running: boolean): Part {
  const r = c.run
  if (!r) {
    return running
      ? skeleton('starting', 5)
      : { status: 'not run yet', rows: [{ key: 'none', mark: 'wait', label: 'Not run here yet', state: '' }] }
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
      log: st.status === 'failed' ? { title: `Last lines of ${stageLabel(st.name)}`, lines: r.log_tail } : null,
    }
  })
  const start = Date.parse(r.started_at)
  return {
    status:
      r.status === 'running'
        ? `running · ${elapsed(now - start)}`
        : r.status === 'passed'
          ? `passed${r.finished_at ? ` · ${duration(Date.parse(r.finished_at) - start)}` : ''}`
          : `failed${r.error ? ` at ${stageLabel(r.error.stage)}` : ''}`,
    rows,
    foot: [
      `Engine ${r.engine}.`,
      r.status === 'passed' && r.not_covered.length ? `Not covered here: ${r.not_covered.join(', ')}.` : null,
    ]
      .filter(Boolean)
      .join(' '),
  }
}

type Past = { id: string; commit: string; checked_at: string; ready: boolean; problems: number }
/** "Checks so far": the checks of the agent's hand-off, oldest first, from the one it was made for. */
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

/** The person's commits conflict with the environment's branch: the agent merges them in its worktree. */
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

function ActionError({ host, status, error }: { host: Host; status: Status; error: WorkerError }) {
  if (MERGE_CODES.has(error.code)) return <MergeConflict host={host} status={status} error={error} />
  if (error.code === 'RUN_BUSY') {
    return (
      <Notice
        variant="info"
        headline={`Already running${error.started_at ? ` since ${clock(error.started_at)}` : ''}`}
        detail="This page updates when the run ends."
      />
    )
  }
  return <ErrorPanel error={error} />
}

function Stage({ host, status: s, scenario, go, narrow, readOnly }: CheckStageProps & { readOnly: boolean }) {
  const hubChecking = useChecking(host) && !readOnly
  const now = useNow(1000)
  const { error, call } = useActions(host)
  const [auto, setAuto] = usePaneState('iii-cloud:check:auto-agent', true)
  const asked = useAsked(s.check?.id)
  const folds = useFolds()
  // Closed, or open on every name the file reads (null), or on only these.
  const [valuesOpen, setValuesOpen] = useState<false | { only: string[] | null }>(false)
  const [importing, setImporting] = useState(false)
  const folder = useFolderNote(s.check?.id) ? FOLDER_NOTE : undefined
  const launching = useLaunch()?.kind === 'launching'
  // The check this page saw running: when it ends not ready, the agent may start (once).
  const watch = useRef<{ base: string | null } | null>(null)

  const c = s.check
  const run = c?.run ?? null
  const running = hubChecking || run?.status === 'running'
  // While a check is in flight and nothing of it has arrived, the shown check is the previous one.
  const fresh = !hubChecking || run?.status === 'running'

  // ponytail: only while the page is open; the worker handing off by itself would cover a closed page.
  useEffect(() => {
    if (readOnly) return
    if (running) {
      watch.current ??= { base: fresh ? null : (c?.id ?? null) }
      return
    }
    const w = watch.current
    if (!w || !c || c.id === w.base) return
    watch.current = null
    if (!auto || c.ready || c.handoff_id || agentOn(s) || askedAt(c.id)) return
    if (openProblems(s).some((p) => p.owner === 'agent')) void handOff(host, s, 'ready')
  })

  // The fix branch goes to GitHub first, so iii cloud can check the commit (as Deploy's first check does).
  const checkAgain = () =>
    void call('check', 'iii-cloud::check', withAppFile(s, { run: true, ...(fixBranch(s) ? { push: true } : {}) }))
  // A skip (or its undo) and "Leave it empty" change what the check finds: it runs again, as after a save.
  const skipNames = async (names: string[], skip: boolean) => {
    const environment_id = s.check?.environment_id ?? s.project?.environment?.id
    if ((await call('skip', 'iii-cloud::values::skip', { environment_id, names, skip })) !== null) checkAgain()
  }
  // O2: "Skip for now" on the workers' config (a warning), or its undo: the status says it at once; the check runs
  // again, as after a values skip, and marks it.
  const skipConfig = async (skip: boolean) => {
    const environment_id = s.check?.environment_id ?? s.project?.environment?.id
    const code = 'LOCAL_CONFIG_NOT_SET'
    if ((await call('skip', 'iii-cloud::problem::skip', { environment_id, code, skip })) !== null) checkAgain()
  }
  const leaveEmpty = async (names: string[]) => {
    const environment_id = s.check?.environment_id ?? s.project?.environment?.id
    const items = names.map((name) => ({ name, from: 'leave_empty' }))
    const manifest = s.check?.manifest || undefined
    if ((await call('save', 'iii-cloud::values::save', { environment_id, manifest, items })) !== null) checkAgain()
  }
  if (!c && !running) {
    if (readOnly) return null
    return (
      <>
        {error ? <ActionError host={host} status={s} error={error} /> : null}
        <EmptyState
          title="Not checked yet"
          description="A check lists what must change before this can run in iii cloud."
          action={{ label: 'Check now', onClick: checkAgain }}
        />
      </>
    )
  }

  const p = s.project
  const envId = c?.environment_id ?? p?.environment?.id ?? ''
  const envRow = p?.environments.find((e) => e.id === envId)
  const env = envRow?.name ?? p?.environment?.name ?? 'the environment'
  const where = p?.workspace?.mode === 'checkout' ? 'in your folder' : 'in a separate copy'
  const phase: Phase = !c
    ? 'checking'
    : c.ready && !running
      ? 'passed'
      : fixing(s)
        ? 'fixing'
        : running
          ? 'checking'
          : 'problems'
  const soFar = run?.status === 'running' ? `${elapsed(now - Date.parse(run.started_at))} so far` : null
  const open = openProblems(s)
  // One deduplicated list: the sections, the summary and the title all count from it.
  const f = findings(s)
  const agentOpen = f.agent.filter((x) => !x.fixed)
  const youOpen = f.person.filter((x) => !x.fixed)
  const past = readOnly ? [] : historyOf(s, c, envId)
  const review = () =>
    go(scenario === 'setup' ? { at: 'setup', step: 'ship' } : { at: 'deploy', env: envId, step: 'review' })
  const ask = () => void handOff(host, s, 'ready')
  const canAsk = !!host.chat?.compose && !launching
  const chosen = (!readOnly && s.override_choices) || {}

  const stops: Stop[] = []
  if (c && !readOnly) {
    if (s.connection.status === 'expired' || signInRequired(s)) {
      stops.push({
        key: 'signin',
        variant: 'alert',
        headline: 'Your iii cloud sign-in expired',
        detail: `${run?.status === 'passed' ? 'The run here finished and passed. ' : ''}The cloud part of the check needs you to sign in again; nothing else is lost.`,
        acts: [
          { label: 'Sign in again', onClick: () => void call('connect', 'iii-cloud::connect', { method: 'browser' }) },
        ],
        lock: 'Sign in again first.',
      })
    }
    // The source the worktree took (a branch, else the checkout's HEAD) has moved since: an older worker says nothing.
    const ws = p?.workspace
    const taken = ws?.mode === 'worktree' ? ws.source_sha : null
    const source = ws?.source ?? null
    const tipHere =
      !source || source === p?.checkout.branch
        ? p?.checkout.head
        : p?.checkout.branches?.find((b) => b.name === source)?.sha
    if (taken && tipHere && sha7(tipHere) !== sha7(taken)) {
      const name = source ?? p?.checkout.branch ?? 'your branch'
      // ponytail: the workspace doesn't record `bring_changes`; its commit's message (project.rs commit_brought) says it
      // was used. A `Workspace.brought` flag from the worker would replace this guess.
      const brought = !!c.commits?.some((x) => x.message.startsWith('Uncommitted changes from '))
      const useCheckout = async () => {
        const used = await call('use', 'iii-cloud::project::use', {
          path: p?.path,
          environment_id: envId,
          mode: ws?.mode,
          from: 'checkout',
          ...(source ? { source } : {}),
          ...(brought ? { bring_changes: true } : {}),
        })
        if (used !== null) checkAgain()
      }
      stops.push({
        key: 'outdated',
        variant: 'info',
        headline: `${name} moved after the check`,
        detail: `${sha7(c.commit)} ${c.ready ? 'passed' : 'was checked'}, but ${name} now ends at ${sha7(tipHere)}. Only a commit that passed can be deployed.`,
        acts: [
          { label: `Check ${sha7(tipHere)}`, onClick: () => void useCheckout(), disabled: running },
          ...(c.ready ? [{ label: `Deploy ${sha7(c.commit)} without it`, onClick: review }] : []),
        ],
        lock: `Check ${sha7(tipHere)} first, or deploy ${sha7(c.commit)} without it.`,
      })
    }
    const err = run?.status === 'failed' ? run.error : null
    if (err?.code === 'DOCKER_MISSING') {
      const cloudN = open.filter((x) => x.source === 'cloud_plan').length
      stops.push({
        key: 'docker',
        variant: 'alert',
        headline: "Docker isn't running on this machine",
        detail: `The run here starts a database and your workers in containers.${cloudPending(c) ? '' : ` The cloud part of the check still ran: ${cloudN ? plural(cloudN, 'problem') : 'no problems'}.`}`,
        acts: [
          { label: 'Run again', onClick: checkAgain, disabled: running },
          { label: 'How to start Docker', href: DOCKER_DOCS },
        ],
      })
    } else if (err && !open.some((x) => x.source === 'check_here')) {
      stops.push({
        key: 'run',
        variant: 'alert',
        headline: `The run here stopped at ${stageLabel(err.stage)}`,
        detail: plainMessage(err),
        acts: [{ label: 'Run again', onClick: checkAgain, disabled: running }],
      })
    }
    const d = s.deploys.find((x) => x.environment_id === envId)
    if (d?.outcome === 'running' && d.check_id !== c.id) {
      stops.push({
        key: 'busy',
        variant: 'info',
        ...deployBusy(d, env),
        acts: [
          {
            label: `Follow #${d.number}`,
            onClick: () => go({ at: 'deploy', env: envId, step: 'follow', deploy: d.id }),
          },
        ],
        lock: `Unlocks when deploy #${d.number} is done.`,
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
    const a = s.agent
    if (a && (a.state === 'done' || a.state === 'stopped') && !c.ready && !running && agentOpen.length) {
      stops.push({
        key: 'stopped',
        variant: 'warn',
        headline: `Your agent couldn't fix ${plural(agentOpen.length, 'problem')}`,
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
    // O1: each console setting the file replaces, as a diff; never a lock (the deploy already takes the file's).
    const choose = (setting: string, choice: OverrideChoice) =>
      call('choose', 'iii-cloud::override::choose', { environment_id: envId, setting, choice })
    for (const o of c.plan.confirmations.overrides) {
      if (chosen[o.setting] === 'file') continue
      const opens = opensToPublic(o)
      const keep = `Make ${appFile(s).path}'s x-iii-cloud.${o.setting} match ${env}'s console value ${JSON.stringify(o.console)}, or drop it from the block, so the deploy keeps the console's.`
      stops.push({
        key: `override:${o.setting}`,
        variant: opens ? 'warn' : 'info',
        headline: `${o.setting}: your file replaces the console's at deploy`,
        detail: (
          <span className="iii-cloud-cs-diff">
            <span>{`console: ${settingWords(o.setting, o.console)}`}</span>
            <span>{`your file: ${settingWords(o.setting, o.file)}`}</span>
            {opens ? <span>Your file makes it public.</span> : null}
            {chosen[o.setting] === 'console' ? <span>You asked your agent to keep the console's.</span> : null}
          </span>
        ),
        acts: [
          { label: opens ? 'Make it public' : "Use my file's", onClick: () => void choose(o.setting, 'file') },
          {
            label: "Keep the console's",
            onClick: () =>
              void handOff(host, s, 'ready', undefined, undefined, keep).then(
                (sent) => sent && choose(o.setting, 'console'),
              ),
            disabled: !canAsk,
          },
        ],
      })
    }
  }
  const lead = stops.some((x) => x.lead !== false)
  const lock = stops.find((x) => x.lock)?.lock

  // The sections (P4): what needs the person, what the agent fixes, what is fixed, what is only worth a look.
  const you = splitWorth(f.person)
  const found = splitWorth(f.agent)
  const needs = you.main.filter((i) => !i.fixed)
  // P3: the run here's missing values are one line (Values owns them); the cloud's keep their grouped row.
  const hereValue = (i: Item) => i.problem?.code === 'LOCAL_VARIABLE_UNSET' && i.problem.source === 'check_here'
  const hereNames = [...new Set(needs.filter(hereValue).map((i) => i.secret ?? i.name ?? i.title))]
  const fixingNow = found.main.filter((i) => !i.fixed)
  const fixedAll = [...f.person, ...f.agent].filter((i) => i.fixed)
  const skipped = f.skipped
  // O1: a console setting the file's value replaces, by the person's choice: one faint line.
  const kept: Item[] = (c?.plan.confirmations.overrides ?? [])
    .filter((o) => chosen[o.setting] === 'file')
    .map((o) => ({
      key: `override:${o.setting}`,
      problem: null,
      title: `At deploy, ${o.setting} comes from your file.`,
      code: null,
      file: null,
      line: null,
      target: null,
      name: null,
      detail: null,
      owner: 'none',
      severity: 'info',
      fixed: false,
      secret: null,
    }))
  // O2: the workers' config leads "Worth a look" (skipped, it is under "Skipped" with Undo).
  const worth = [
    ...you.worth.filter(configAction),
    ...you.worth.filter((i) => !configAction(i)),
    ...found.worth,
    ...f.notes,
    ...kept,
  ]
  const working = phase === 'fixing'

  // The header: title and state chip, where it was checked, the app file; the forward action on its right.
  const commit = c ? `Commit ${sha7(c.commit)}${run && isDirty(run.commit) ? ' with uncommitted changes' : ''}` : null
  // Only what blocks is a thing to fix (a skipped SECRET_MISSING still blocks); warnings never count.
  const count = [...agentOpen, ...youOpen].filter((i) => i.severity === 'block').length
  // iii cloud accepts it and nothing blocks, but the run here never ran on this commit (a failed run has its stop).
  const runPending = phase === 'problems' && !!c && c.plan.status === 'clean' && !count && !run
  // D6: one line that adds up, from the sections' own counts.
  const total = needs.length + fixingNow.length + fixedAll.length
  const progress = (
    <>
      <span className="iii-cloud-cs-total">{plural(total, 'problem')}</span>
      {`: ${needs.length} ${needs.length === 1 ? 'needs' : 'need'} you · ${
        working && !fixingNow.length ? 'your agent is checking the run here' : `your agent is on ${fixingNow.length}`
      } · ${fixedAll.length} fixed${skipped.length ? ` · ${skipped.length} skipped` : ''}`}
    </>
  )
  const atLeast = !!c && (c.plan.at_least || cloudPending(c))
  const checked = c
    ? cloudPending(c)
      ? run
        ? 'checked here · iii cloud check pending'
        : 'iii cloud check pending'
      : run
        ? 'checked on iii cloud and here'
        : 'checked on iii cloud'
    : null
  const heads: Record<Phase, { title: string; sub: (string | null)[]; chip: ReactNode }> = {
    checking: {
      title: 'Checking before deploy',
      sub: [commit, `${where} of your code`, soFar],
      chip: <StatusChip live>Checking</StatusChip>,
    },
    problems: {
      title: readOnly
        ? `${plural(count, 'thing')} to fix in ${c ? sha7(c.commit) : ''}`
        : count
          ? `${atLeast ? 'At least ' : ''}${plural(count, 'thing')} to fix before ${env} can get this`
          : `Not ready for ${env} yet`,
      sub: [commit, checked],
      chip: <StatusChip tone="warning">Not ready</StatusChip>,
    },
    fixing: {
      title: 'Your agent is fixing what the check found',
      sub: [total ? null : 'Nothing is deployed while it works'],
      chip: <StatusChip live>{running ? 'Checking again' : 'Fixing'}</StatusChip>,
    },
    passed: {
      title: readOnly ? `${c ? sha7(c.commit) : ''} passed` : `Ready to deploy to ${env}`,
      sub: [
        c
          ? `${sha7(c.commit)} passed on ${run ? 'iii cloud and here' : 'iii cloud'} at ${clock(run?.finished_at ?? c.checked_at)}`
          : null,
      ],
      chip: <StatusChip tone="success">Passed</StatusChip>,
    },
  }
  const h = runPending
    ? {
        title: 'iii cloud accepts this commit; run it here to finish the check',
        sub: ['The run here starts your app like iii cloud does: install, build, release, then your workers.'],
        chip: <StatusChip tone="warning">Run here pending</StatusChip>,
      }
    : heads[phase]
  const file = appFile(s)
  const sum = c?.plan.summary
  const fileNote = [
    file.note ?? `the file ${env} runs`,
    sum ? `${plural(sum.workers, 'worker')} in ${plural(sum.groups.length, 'group')}` : null,
  ]
    .filter(Boolean)
    .join(' · ')
  const pending = c && phase !== 'checking' ? cloudPendingNote(s) : null

  // A record shows everything it has; a live check shows the parts until there is something found.
  const foundN = f.agent.length + f.notes.length
  const showParts = readOnly || phase === 'checking' || foundN === 0
  const cloud = c && fresh ? cloudPart(s, c) : skeleton('checking', 4)
  const here = c && fresh ? runPart(s, c, now, running) : skeleton('next', 5)

  const logLines = c?.run?.log_tail ?? []
  const ctx: ListCtx = {
    host,
    status: s,
    folds,
    working: workingOnIds(s),
    log: logLines,
    logKey: logLines.length
      ? [...f.agent, ...f.notes].find(
          (i) => !i.fixed && i.problem?.source === 'check_here' && !codeLines(i.problem).length,
        )?.key
      : undefined,
    values: readOnly || !envId ? null : (only) => setValuesOpen({ only: only ?? null }),
    config:
      readOnly || !envId
        ? null
        : {
            copy: () => setImporting(true),
            skip: (on) => void skipConfig(on),
            url: envPageUrl(s, 'config'),
          },
    env,
    parts: true,
    skip: readOnly || !envId ? null : (names, on) => void skipNames(names, on),
    leaveEmpty: readOnly || !envId ? null : (names) => void leaveEmpty(names),
  }
  // P4: what needs the person opens when there is any; the agent's list while it works (on a desktop, or when
  // nothing waits on the person); the rest start folded.
  const foundDefault = working || (!narrow && fixingNow.length > 0)

  const nowAt = past.findIndex((x) => running && x.id === c?.id)
  const waitFor = nowAt >= 0 ? nowAt + 1 : hubChecking ? past.length + 1 : 0
  const results = past.map((x, i) =>
    i === nowAt ? 'running' : x.ready ? 'passed' : x.problems ? plural(x.problems, 'problem') : 'no problems',
  )
  const latest = past.at(-1)
  const pastNote = latest
    ? past.length - 1 === nowAt
      ? `check ${nowAt + 1} running${soFar ? ` · ${soFar}` : ''}`
      : `last ${clock(latest.checked_at)}, ${results.at(-1)}`
    : null
  // P1: the one forward action, in the header; its note (why it is off, or what it does) is the faint line under the title.
  let act: ReactNode = null
  let note: string | null = null
  if (phase === 'passed' && !lock) {
    act = (
      <Button variant={lead ? 'secondary' : 'primary'} onClick={review}>
        Review and deploy
      </Button>
    )
    note = 'Only the commit that passed can be deployed.'
  } else if (runPending) {
    act = (
      <Button variant={lead ? 'secondary' : 'primary'} disabled={running} onClick={checkAgain}>
        Run it here
      </Button>
    )
  } else if (phase === 'problems') {
    // With the agent stopped, its panel holds "Ask your agent to finish".
    if (agentOpen.length && !stops.some((x) => x.key === 'stopped')) {
      const again = !!asked || !!s.agent
      act = (
        <Button variant={again || lead ? 'secondary' : 'primary'} disabled={!canAsk} onClick={ask}>
          {sendLabel(s, 'ready', again ? 'Start over with your agent' : 'Fix with your agent')}
        </Button>
      )
      note = asked
        ? `Asked your agent at ${clock(asked)}. This page updates when it checks again.${folder ? ` ${folder}` : ''}`
        : 'Opens its chat beside this page. The check runs again after each fix.'
    } else if (!agentOpen.length) {
      act = (
        <Button variant="secondary" disabled={running} onClick={checkAgain}>
          Check again
        </Button>
      )
      note = needs.length ? 'Check again once you have done what is below.' : null
    }
  } else {
    act = (
      <Button variant="secondary" disabled>
        Review and deploy
      </Button>
    )
    note =
      phase === 'passed' ? (lock ?? null) : phase === 'fixing' && waitFor ? `Waiting for check ${waitFor}.` : UNLOCKS
  }
  // D8: what the person's section says under its rows.
  const youFoot = !needs.length
    ? undefined
    : working
      ? needs.some((i) => i.severity === 'block')
        ? "Your agent can't do these: they need values only you have. It keeps fixing the rest."
        : undefined
      : needs.some((i) => !personAction(i))
        ? 'Your agent asks you about these in its chat after you hand it over.'
        : undefined

  return (
    <div className="iii-cloud-cs" data-narrow={narrow ? '' : undefined}>
      <StepHeader
        title={h.title}
        chip={<span aria-live="polite">{h.chip}</span>}
        sub={
          <>
            <p className="iii-cloud-cs-sub">{working && total ? progress : h.sub.filter(Boolean).join(' · ')}</p>
            {working ? null : (
              <p className="iii-cloud-desc">
                App file <Mono>{file.path}</Mono>
                {fileNote ? ` · ${fileNote}` : null}
              </p>
            )}
          </>
        }
        forward={readOnly ? null : act}
        reason={readOnly ? null : note}
      />
      {pending ? <Notice variant="info" headline="iii cloud check pending" detail={pending} /> : null}
      {error ? <ActionError host={host} status={s} error={error} /> : null}
      {stops.map((x) => (
        <Notice
          key={x.key}
          className="iii-cloud-cs-stop"
          variant={x.variant}
          headline={x.headline}
          detail={x.detail}
          action={
            <span className="iii-cloud-row">
              {x.acts.map((a, i) =>
                a.href ? (
                  <Button key={a.label} asChild size="sm" variant="secondary">
                    <a href={a.href} target="_blank" rel="noreferrer">
                      {a.label}
                    </a>
                  </Button>
                ) : (
                  <Button
                    key={a.label}
                    size="sm"
                    variant={i === 0 && x.lead !== false ? 'primary' : 'secondary'}
                    disabled={a.disabled}
                    onClick={a.onClick}
                  >
                    {a.label}
                  </Button>
                ),
              )}
            </span>
          }
        />
      ))}
      {working && s.agent ? <AgentRow host={host} agent={s.agent} note={folder} branch={p?.workspace?.branch} /> : null}
      {showParts ? (
        <div className="iii-cloud-cs-parts">
          <Section
            id="cloud"
            folds={folds}
            byDefault
            title="iii cloud accepts it"
            note={cloud.status}
            foot={cloud.foot}
          >
            <Rows rows={cloud.rows} />
          </Section>
          <Section
            id="here"
            folds={folds}
            byDefault
            title="It runs here like the cloud"
            note={here.status}
            foot={here.foot}
          >
            <Rows rows={here.rows} />
          </Section>
        </div>
      ) : null}
      {phase === 'checking' && !readOnly ? (
        <div className="iii-cloud-cs-leave">
          <span className="iii-cloud-grow">You can leave this page. The console tells you when the check is done.</span>
          {host.chat?.compose ? (
            <Checkbox
              label="Start my agent if it finds problems"
              checked={auto}
              onChange={(e) => setAuto(e.target.checked)}
            />
          ) : null}
        </div>
      ) : null}
      {phase !== 'checking' && !readOnly && needs.length ? (
        <Section
          id="you"
          folds={folds}
          byDefault
          title="Needs you"
          count={needs.length}
          note="the agent never sees secrets"
          foot={youFoot}
        >
          <ItemList
            items={needs.filter((i) => !hereValue(i))}
            ctx={{ ...ctx, parts: false }}
            after={hereNames.length ? <HereValuesRow names={hereNames} ctx={ctx} /> : null}
          />
        </Section>
      ) : null}
      {phase !== 'checking' && fixingNow.length ? (
        <Section
          id="found"
          folds={folds}
          byDefault={foundDefault}
          title={working ? 'Your agent is fixing' : 'What the check found'}
          count={fixingNow.length}
          note={working ? null : `your agent fixes these ${where}`}
          foot={phase === 'problems' && c?.plan.at_least ? AT_LEAST : undefined}
        >
          <ItemList
            items={fixingNow}
            ctx={
              working ? { ...ctx, working: new Set(fixingNow.flatMap((i) => (i.problem ? [i.problem.id] : []))) } : ctx
            }
          />
        </Section>
      ) : null}
      {phase !== 'checking' && fixedAll.length ? (
        <Section id="fixed" folds={folds} byDefault={false} title="Fixed" count={fixedAll.length}>
          <ItemList items={fixedAll} ctx={ctx} />
        </Section>
      ) : null}
      {phase !== 'checking' && !readOnly && skipped.length ? (
        <Section
          id="skipped"
          folds={folds}
          byDefault={false}
          title="Skipped"
          count={skipped.length}
          foot={[
            'Skipped for now.',
            skipped.some((i) => !configAction(i)) ? 'Set them in Values whenever you want.' : null,
            skipped.some(configAction) ? "Copy the workers' config from Settings whenever you want." : null,
          ]
            .filter(Boolean)
            .join(' ')}
        >
          <ItemList items={skipped} ctx={{ ...ctx, parts: false, undo: true }} />
        </Section>
      ) : null}
      {phase !== 'checking' && worth.length ? (
        <Section
          id="worth"
          folds={folds}
          byDefault={false}
          title="Worth a look"
          count={worth.length}
          note="won't block the deploy"
        >
          <ItemList items={worth} ctx={ctx} />
        </Section>
      ) : null}
      {phase !== 'checking' && past.length ? (
        <Section
          id="past"
          folds={folds}
          byDefault={false}
          title="Checks so far"
          count={past.length}
          note={pastNote}
          foot={`Each commit your agent makes ${where} starts a new check by itself. Nothing is deployed while it works.`}
        >
          <ol className="iii-cloud-cs-hist">
            {past.map((x, i) => {
              const live = i === nowAt
              const what = i === 0 ? (scenario === 'setup' ? 'Your code' : 'Your commits') : 'After a fix'
              return (
                <li key={x.id} data-now={live ? '' : undefined}>
                  <Mono>{sha7(x.commit)}</Mono>
                  <span>{`${what} · ${results[i]}`}</span>
                  <span className="iii-cloud-cs-state">{live ? soFar : clock(x.checked_at)}</span>
                </li>
              )
            })}
          </ol>
        </Section>
      ) : null}
      {valuesOpen ? (
        <ValuesDialog
          host={host}
          status={s}
          manifest={c?.manifest || null}
          only={valuesOpen.only ?? undefined}
          narrow={narrow}
          after="and check again"
          disabled={running}
          onClose={() => setValuesOpen(false)}
          onSaved={checkAgain}
        />
      ) : null}
      {importing ? (
        <ConfigImportDialog
          host={host}
          status={s}
          envId={envId}
          env={env}
          dir={configDir(open.find((x) => x.code === 'LOCAL_CONFIG_NOT_SET')) ?? 'the config folder'}
          after="then the check runs again"
          onClose={() => setImporting(false)}
          onApplied={checkAgain}
        />
      ) : null}
    </div>
  )
}

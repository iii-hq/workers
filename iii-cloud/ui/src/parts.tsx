import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  Chip,
  ConfirmDialog,
  IconButton,
  Panel,
  PanelBody,
  PanelHeader,
  StatusDot,
  StatusPanel,
  type StatusPanelProps,
  Toolbar,
} from '@iii-dev/console-ui'
import {
  ArrowLeft,
  CircleAlert,
  CircleCheck,
  ExternalLink,
  Info,
  Loader2,
  RefreshCw,
  TriangleAlert,
} from 'lucide-react'
import { Fragment, type ReactNode, useEffect, useState } from 'react'
import { clock } from './asked'
import { hubOf, useActions, useLocked, useStale } from './hub'
import { envName, needsAction, plainDetail, plainMessage, type RowTone, type Tone, whatWaits } from './model'
import type { PathItem, Route } from './route'
import type { Deploy, Project, Status, WorkerError } from './types'

export function useNow(everyMs = 1000): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), everyMs)
    return () => clearInterval(t)
  }, [everyMs])
  return now
}

/**
 * A step's header, the same on every Setup and Deploy step: the title and its state chip; on the title's line,
 * right-aligned, the step's one forward action with Cancel or Back (a link) left of it; under the title the sub
 * line, then a faint note (why the action is off, or what it does). There is no copy of the action below. A
 * narrow pane puts the action full width right under the title.
 */
export function StepHeader({
  title,
  chip,
  sub,
  back,
  forward,
  reason,
}: {
  title: ReactNode
  chip?: ReactNode
  sub?: ReactNode
  /** A link left of the forward action: Cancel, Back, or a second way on. */
  back?: ReactNode
  forward?: ReactNode
  reason?: ReactNode
}) {
  return (
    <div className="iii-cloud-stephead">
      <div className="iii-cloud-stephead-title">
        <h2 className="iii-cloud-h">{title}</h2>
        {chip}
      </div>
      {back ? <div className="iii-cloud-stephead-back">{back}</div> : null}
      {forward ? <div className="iii-cloud-stephead-fwd">{forward}</div> : null}
      {sub ? <div className="iii-cloud-stephead-sub">{sub}</div> : null}
      {reason ? <p className="iii-cloud-stephead-reason">{reason}</p> : null}
    </div>
  )
}

/** The link left of a step's forward action (Cancel, Back, Check without saving). */
export function StepLink({
  onClick,
  disabled,
  children,
}: {
  onClick: () => void
  disabled?: boolean
  children: ReactNode
}) {
  return (
    <button type="button" className="iii-cloud-linkbtn iii-cloud-small" disabled={disabled} onClick={onClick}>
      {children}
    </button>
  )
}

const DOT: Record<RowTone, { tone: 'ok' | 'alert' | 'warn' | 'ghost' | 'accent'; pulse?: boolean }> = {
  ok: { tone: 'ok' },
  alert: { tone: 'alert' },
  warn: { tone: 'warn' },
  live: { tone: 'accent', pulse: true },
  edge: { tone: 'ghost' },
}

export function Dot({ tone }: { tone: RowTone | 'ghost' }) {
  const d = tone === 'ghost' ? DOT.edge : DOT[tone]
  return <StatusDot tone={d.tone} pulse={d.pulse} aria-hidden />
}

const CHIP_TONE = { success: 'ok', warning: 'warn', danger: 'alert', neutral: null } as const

/** A neutral chip with a status dot: the word stays ink, only the dot carries the status (AA). */
export function StatusChip({
  tone = 'neutral',
  live,
  children,
}: {
  tone?: 'success' | 'warning' | 'danger' | 'neutral'
  live?: boolean
  children: ReactNode
}) {
  const dot = live ? 'live' : CHIP_TONE[tone]
  return (
    <Chip>
      {dot ? <Dot tone={dot} /> : null}
      {children}
    </Chip>
  )
}

/** Spinner for a running check (the shared spin recipe). */
export const Spinner = () => <Loader2 className="iii-ui-spin" aria-hidden />

export const Mono = ({ children }: { children: ReactNode }) => <span className="iii-cloud-mono">{children}</span>

/** A sentence with each short commit sha (seven hex characters, at least one a digit) set in mono. */
export function monoShas(text: string): ReactNode {
  return text
    .split(/\b((?=[0-9a-f]*\d)[0-9a-f]{7})\b/)
    .map((part, i) => (i % 2 ? <Mono key={part + i}>{part}</Mono> : part))
}

const TINT: Partial<Record<Tone, string>> = { ok: 'ok', alert: 'alert', warn: 'warn' }

/** A state word with its dot on the status tint (`deployState`, `liveOf`): the word stays ink. */
export function StatePill({ state }: { state: { label: string; tone: Tone } }) {
  return (
    <Chip className="iii-cloud-state" data-tint={TINT[state.tone]}>
      <Dot tone={state.tone} />
      {state.label}
    </Chip>
  )
}

/**
 * A card with a header: the title (`onTitle` makes it a link), its `count` beside it, a right-hand `note`
 * (faint) and `end` (an action); then the body (`flush`: a list or table fills it). The look of "You",
 * "Your agent" and "Deployments".
 */
export function SectionCard({
  title,
  count,
  note,
  onTitle,
  end,
  flush,
  children,
}: {
  title: string
  count?: number
  note?: ReactNode
  onTitle?: () => void
  end?: ReactNode
  flush?: boolean
  children: ReactNode
}) {
  return (
    <Panel className="iii-cloud-panel" role="region" aria-label={title}>
      <PanelHeader className="iii-cloud-panel-head">
        <span className="iii-cloud-row">
          {onTitle ? (
            <button type="button" className="iii-cloud-titlelink" onClick={onTitle}>
              {title}
            </button>
          ) : (
            <span>{title}</span>
          )}
          {count != null ? <span className="iii-cloud-count">{count}</span> : null}
        </span>
        {note || end ? (
          <span className="iii-cloud-panel-end">
            {note}
            {end}
          </span>
        ) : null}
      </PanelHeader>
      {flush ? children : <PanelBody className="iii-cloud-panel-body">{children}</PanelBody>}
    </Panel>
  )
}

/**
 * One row of a card of facts: a `label`, its value (`children`) with a faint `note` under it, and an `action`
 * at the end, usually "Change" (`onChange`). The confirm screen and the secrets page both read this way.
 */
export function DetailRow({
  label,
  note,
  action,
  onChange,
  children,
}: {
  label: string
  note?: ReactNode
  action?: ReactNode
  /** Shorthand for the "Change" action. */
  onChange?: () => void
  children: ReactNode
}) {
  return (
    <div className="iii-cloud-detail">
      <span className="iii-cloud-detail-label">{label}</span>
      <span className="iii-cloud-detail-value">
        {children}
        {note ? <span className="iii-cloud-detail-note">{note}</span> : null}
      </span>
      {action ??
        (onChange ? (
          <button type="button" className="iii-cloud-linkbtn iii-cloud-small" onClick={onChange}>
            Change
          </button>
        ) : null)}
    </div>
  )
}

/** The one primary action of a page header ("Deploy to dev", "Continue"): the title's line, full width on a phone. */
export function PageAction({
  onClick,
  disabled,
  children,
}: {
  onClick: () => void
  disabled?: boolean
  children: ReactNode
}) {
  return (
    <Button variant="primary" className="iii-cloud-pageaction" disabled={disabled} onClick={onClick}>
      {children}
    </Button>
  )
}

/** A small secondary button that opens `href` in a new tab (GitHub, iii cloud, the app). */
export function LinkButton({ href, children }: { href: string; children: ReactNode }) {
  return (
    <Button size="sm" variant="secondary" asChild>
      <a href={href} target="_blank" rel="noreferrer">
        {children}
        <ExternalLink />
      </a>
    </Button>
  )
}

/** A worker error as a panel: plain headline in ink, the fix or explanation as detail, never the code. */
export function ErrorPanel({
  error,
  variant = 'alert',
  action,
}: {
  error: WorkerError
  variant?: 'alert' | 'warn'
  action?: ReactNode
}) {
  return <Notice variant={variant} headline={plainMessage(error)} detail={plainDetail(error)} action={action} />
}

const NOTICE_ICON = { info: <Info />, success: <CircleCheck />, warn: <TriangleAlert />, alert: <CircleAlert /> }

/** The shared StatusPanel with its icon; the scoped class keeps the headline ink (only the tint and icon carry the status). */
export function Notice({ variant = 'info', className, headline, ...rest }: StatusPanelProps) {
  return (
    <StatusPanel
      variant={variant}
      icon={NOTICE_ICON[variant]}
      className={className}
      {...rest}
      headline={<span className="iii-cloud-ink">{headline}</span>}
    />
  )
}

/**
 * The one path bar at the top of every area's PageMain: the ancestors as links, " / ", the place itself as
 * the page's heading. Narrow: a back button to the parent (the last ancestor that goes somewhere) and the
 * place only. `children` follow the name (a chip, the address); `end` holds "as of", Refresh and actions.
 */
export function PathBar({
  path,
  go,
  narrow,
  rule,
  children,
  end,
}: {
  path: PathItem[]
  go: (r: Route) => void
  narrow?: boolean
  /** A hairline under the bar, when no tab row follows it. */
  rule?: boolean
  children?: ReactNode
  end?: ReactNode
  /** Not read: no board has a deploy pill in the bar (Projects' rows say what is going on); callers may stop passing them. */
  host?: Host
  status?: Status
  route?: Route
}) {
  const here = path[path.length - 1]
  const up = path.slice(0, -1)
  const parent = up.filter((p) => p.to).at(-1)
  return (
    <Toolbar aria-label={here.label} className="iii-cloud-envhead" data-rule={rule ? '' : undefined} end={end}>
      {narrow ? (
        <>
          {parent?.to ? (
            <IconButton label={`Back to ${parent.label}`} variant="ghost" onClick={() => go(parent.to as Route)}>
              <ArrowLeft />
            </IconButton>
          ) : null}
          <h2 className="iii-cloud-head-title iii-cloud-head-name">{here.label}</h2>
        </>
      ) : (
        <nav aria-label="Breadcrumb" className="iii-cloud-crumbs">
          {up.map((p) => (
            <Fragment key={p.to?.at ?? p.label}>
              {p.to ? (
                <button type="button" className="iii-cloud-crumbs-up" onClick={() => go(p.to as Route)}>
                  {p.label}
                </button>
              ) : (
                <span>{p.label}</span>
              )}
              <span aria-hidden> / </span>
            </Fragment>
          ))}
          <h2 className="iii-cloud-head-title" aria-current="page">
            {here.label}
          </h2>
        </nav>
      )}
      {children}
    </Toolbar>
  )
}

/**
 * A deploy going on, wherever it was made. `other`: not the folder in use (switching takes `path`; none known:
 * only iii cloud can show it). `name` and `env` are null when the worker never knew them.
 */
export type Moving = {
  deploy: Deploy
  other: boolean
  name: string | null
  env: string | null
  path: string | null
  cloudProject: string | null
}

/**
 * Every deploy going on (running, or waiting on the person) of every project and environment, newest first:
 * `running_deploys`; a worker without it says only the selected environment's (`deploys`).
 */
export function moving(s: Status): Moving[] {
  const p = s.project
  const envOf = (id: string) => p?.environments.find((e) => e.id === id)?.name ?? null
  const on = (d: Deploy) => d.outcome === 'running' || needsAction(d)
  if (!s.running_deploys)
    return s.deploys.filter(on).map((d) => ({
      deploy: d,
      other: false,
      name: null,
      env: envOf(d.environment_id),
      path: p?.path ?? null,
      cloudProject: p?.cloud_project?.id ?? null,
    }))
  return s.running_deploys
    .filter((r) => on(r.deploy))
    .map((r) => {
      // An environment of this project, made from this folder (or a folder the worker never knew): this project's.
      const mine = !!envOf(r.deploy.environment_id) && (!r.path || r.path === p?.path)
      return {
        deploy: r.deploy,
        other: !mine,
        name: mine ? null : (r.project_name ?? r.repository),
        env: r.environment_name ?? envOf(r.deploy.environment_id),
        path: r.path,
        cloudProject: r.cloud_project_id,
      }
    })
}

/** "as of 14:40" and Refresh: reads the status again, and with it every cloud route on the page. */
export function Fresh({ host, status }: { host: Host; status: Status; narrow?: boolean }) {
  return (
    <>
      {status.refreshed_at ? <span className="iii-cloud-desc">as of {clock(status.refreshed_at)}</span> : null}
      <IconButton label="Refresh" variant="ghost" onClick={() => void hubOf(host).refresh(true)}>
        <RefreshCw />
      </IconButton>
    </>
  )
}

/**
 * Taking a project up again: `project::use`, then its home (or `then`, such as a deploy's follow page, once the
 * status has the project). Something waiting on the person asks first (`useLeave`). An environment that is gone
 * falls back to the project's default: the home says so. `switching` is the key of the place on its way; render
 * `dialog` once.
 */
export function useSwitch(host: Host, status: Status, go: (r: Route) => void) {
  const { busy, error, call } = useActions(host)
  const { leave, dialog } = useLeave(host, status)
  const [target, setTarget] = useState<string | null>(null)
  const switchTo = (
    key: string,
    name: string,
    payload: Record<string, unknown>,
    envName?: string | null,
    then?: Route,
  ) =>
    leave(name, () => {
      setTarget(key)
      void call<Project>('use', 'iii-cloud::project::use', payload).then(async (out) => {
        setTarget(null)
        if (out === null) return
        const note = out?.fell_back_from
          ? `${envName ?? 'That environment'} no longer exists; showing ${out.environment?.name ?? 'the default environment'}`
          : undefined
        if (then && !note) {
          await hubOf(host).refresh(false)
          return go(then)
        }
        go(note ? { at: 'env', env: '', note } : { at: 'env', env: '' })
      })
    })
  return { switching: busy === 'use' ? target : null, error, switchTo, leave, dialog }
}

/** The worker stopped answering: the page keeps its last status, and saves stay locked (`useLocked`). */
export function StaleNotice({ host, status }: { host: Host; status: Status }) {
  if (!useStale(host)) return null
  return (
    <Notice
      variant="warn"
      headline={`iii cloud isn't answering · showing what it said${status.refreshed_at ? ` at ${clock(status.refreshed_at)}` : ''}`}
      detail="Statuses are greyed out until it answers. Deploying and saves stay locked."
      action={
        <Button variant="secondary" size="sm" onClick={() => void hubOf(host).refresh(true)}>
          Try again
        </Button>
      }
    />
  )
}

const sentence = (parts: string[]) => {
  const text = parts.length > 1 ? `${parts.slice(0, -1).join(', ')} and ${parts.at(-1)}` : (parts[0] ?? '')
  return text.charAt(0).toUpperCase() + text.slice(1)
}

/**
 * Leaving the selected environment while something waits on the person asks first (`whatWaits`): the
 * agent's question, a secret, a held deploy. `leave(to, run)` runs at once when nothing waits; render `dialog`.
 * A confirm while the page went out of date or wants a fresh sign-in (`useLocked`) only closes the dialog.
 */
export function useLeave(host: Host, status: Status) {
  const locked = useLocked(host, status)
  const [ask, setAsk] = useState<{ to: string; run: () => void } | null>(null)
  const waits = whatWaits(status)
  const here = envName(status) || status.project?.repository || 'this project'
  const leave = (to: string, run: () => void) => (waits.length ? setAsk({ to, run }) : run())
  const dialog = (
    <ConfirmDialog
      open={!!ask}
      onOpenChange={(open) => {
        if (!open) setAsk(null)
      }}
      title={`Leave ${here} while it waits for you?`}
      description={`${sentence(waits)}. It keeps waiting; open it again to answer.`}
      confirmLabel={`Open ${ask?.to ?? ''}`}
      cancelLabel="Stay"
      onConfirm={() => {
        const run = ask?.run
        setAsk(null)
        if (!locked) run?.()
      }}
    />
  )
  return { leave, dialog }
}

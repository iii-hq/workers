import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  Checkbox,
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  EmptyState,
  IconButton,
  Input,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  SegmentedControl,
  Skeleton,
} from '@iii-dev/console-ui'
import { errorCode, errorMessage } from '@iii-dev/console-ui/format'
import { ArrowLeft, Rocket } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { AgentRow } from './AgentRow'
import { cloudRead, cloudWrite, useCloud } from './api'
import { clock } from './asked'
import { type DeploymentList, Section, StateChip, useDeployments } from './EnvOverview'
import { FailureCard, failureTail, LogLines } from './FailureCard'
import { hubOf, useLocked } from './hub'
import {
  composeFile,
  deployRows,
  deploySource,
  deployWord,
  isFailedDeploy,
  isGone,
  needsAction,
  OUTCOME_LABEL,
  type RowTone,
  roleOf,
  servingDeploy,
  sha7,
  workerError,
} from './model'
import { Dot, ErrorPanel, Mono, Notice } from './parts'
import type { EnvTabProps } from './route'
import type { Deploy, Status, WorkerError } from './types'

type Preview = { returns: string[]; stays: string[]; side_effects: string[]; queue: string }
type LogPage = { lines: { message: string }[] }

/** An environment's Deploys tab: the list beside one deploy (Mng2), rollback (Mng7), one pane on a phone (Mng10). */
export function EnvDeploys({ host, status, route, go, narrow, envId }: EnvTabProps) {
  const deploys = status.deploys.filter((d) => d.environment_id === envId)
  const list = useDeployments(host, status, envId).data
  const open = (deploy?: string) => go({ at: 'env', env: envId, tab: 'deploys', deploy })
  const env = status.project?.environment?.name ?? 'this environment'
  if (deploys.length === 0) {
    return (
      <EmptyState
        icon={Rocket}
        title={`No deploys of ${env} yet`}
        description="Deploy from this machine checks your code on iii cloud and here, then deploys it."
        action={{ label: 'Deploy from this machine', onClick: () => go({ at: 'deploy', env: envId }) }}
      />
    )
  }
  const live = list?.deployments.find((d) => d.current)?.id ?? servingDeploy(status, envId)?.id ?? null
  const picked = route.deploy ? deploys.find((d) => d.id === route.deploy) : narrow ? undefined : deploys[0]
  const detail = picked ? (
    <DeployDetail
      key={picked.id}
      {...{ host, status, envId, deploys, list, live, narrow, open }}
      deploy={picked}
      reviewWaiting={() => go({ at: 'deploy', env: envId, step: 'review' })}
    />
  ) : route.deploy ? (
    <EmptyState
      title="Deploy not found"
      description={`It is not among the latest 20 deploys of ${env}.`}
      action={{ label: 'All deploys', onClick: () => open() }}
    />
  ) : null
  if (narrow && route.deploy) return detail
  const rows = <DeployList deploys={deploys} live={live} selected={picked?.id} open={open} />
  return narrow ? (
    rows
  ) : (
    <div className="iii-cloud-deploys">
      {rows}
      <div className="iii-cloud-dpane">{detail}</div>
    </div>
  )
}

/** The list's word for a deploy: the one serving is Live, an earlier live one Superseded. */
function wordOf(d: Deploy, live: string | null): { label: string; tone: RowTone } {
  if (d.id === live) return { label: 'Live', tone: 'ok' }
  const label = d.outcome === 'ready' && !isGone(d) ? OUTCOME_LABEL.replaced : deployWord(d)
  const tone = needsAction(d) ? 'warn' : d.outcome === 'running' ? 'live' : isFailedDeploy(d) ? 'alert' : 'edge'
  return { label, tone }
}

const Word = ({ d, live }: { d: Deploy; live: string | null }) => <StateChip live={wordOf(d, live)} />

function dayOf(iso: string): string {
  const day = (t: Date) => new Date(t.getFullYear(), t.getMonth(), t.getDate()).getTime()
  const at = new Date(iso)
  const back = Math.round((day(new Date()) - day(at)) / 86_400_000)
  return back === 0 ? 'Today' : back === 1 ? 'Yesterday' : at.toLocaleDateString([], { day: 'numeric', month: 'short' })
}

const message = (d: Deploy) => d.commit.message?.split('\n')[0]

/** The detail's headline: the commit's subject, else "Deploy #5 is live" (where it came from stays in the meta). */
function headline(d: Deploy, live: string | null): string {
  const said = message(d)
  if (said) return said
  const how =
    d.id === live
      ? 'is live'
      : isGone(d)
        ? 'no longer exists'
        : needsAction(d)
          ? 'needs your action'
          : d.outcome === 'running'
            ? 'is deploying'
            : isFailedDeploy(d)
              ? 'failed'
              : d.outcome === 'canceled'
                ? 'was canceled'
                : 'was superseded'
  return `Deploy #${d.number} ${how}`
}

function DeployList({
  deploys,
  live,
  selected,
  open,
}: {
  deploys: Deploy[]
  live: string | null
  selected?: string
  open: (id: string) => void
}) {
  const days: { day: string; items: Deploy[] }[] = []
  for (const d of deploys) {
    const day = dayOf(d.created_at)
    if (days.at(-1)?.day === day) days.at(-1)?.items.push(d)
    else days.push({ day, items: [d] })
  }
  // A flush column under the tabs: "#6", its word and the time, then the commit on the next line.
  return (
    <List aria-label="Deploys" className="iii-cloud-dlist">
      {days.map((g) => (
        <ListGroup key={g.day}>
          <ListGroupLabel>{g.day}</ListGroupLabel>
          {g.items.map((d) => (
            <ListItem
              key={d.id}
              label={
                <span className="iii-cloud-drow">
                  <span className="iii-cloud-mono iii-cloud-dnum">#{d.number}</span>
                  <Word d={d} live={live} />
                  <span className="iii-cloud-grow" />
                  <span className="iii-cloud-mono iii-cloud-faint">{clock(d.created_at)}</span>
                </span>
              }
              description={
                <>
                  <Mono>{sha7(d.commit.sha)}</Mono> {message(d) ?? deploySource(d)}
                </>
              }
              selected={d.id === selected}
              aria-current={d.id === selected ? 'true' : undefined}
              onClick={() => open(d.id)}
            />
          ))}
        </ListGroup>
      ))}
    </List>
  )
}

/** When it ran: the live one since it finished, an earlier live one until the next one did, else when it started. */
function whenOf(d: Deploy, deploys: Deploy[], live: string | null): string {
  if (d.outcome !== 'ready' || !d.finished_at) return clock(d.created_at)
  const next = [...deploys]
    .reverse()
    .find((x) => x.outcome === 'ready' && Date.parse(x.created_at) > Date.parse(d.created_at))
  return d.id !== live && next?.finished_at
    ? `live ${clock(d.finished_at)} to ${clock(next.finished_at)}`
    : `live since ${clock(d.finished_at)}`
}

function DeployDetail({
  host,
  status,
  envId,
  deploy: d,
  deploys,
  list,
  live,
  narrow,
  open,
  reviewWaiting,
}: {
  host: Host
  status: Status
  envId: string
  deploy: Deploy
  deploys: Deploy[]
  list: DeploymentList | null
  live: string | null
  narrow: boolean
  open: (id?: string) => void
  reviewWaiting: () => void
}) {
  const project = status.project
  const env = project?.environment?.name ?? 'the environment'
  const failed = isFailedDeploy(d)
  const by = list?.deployments.find((x) => x.id === d.id)?.triggered_by.user_id
  const byYou = !!by && by === status.connection.user?.id
  const earlierLive = deploys.some((x) => x.outcome === 'ready' && Date.parse(x.created_at) < Date.parse(d.created_at))
  const servingNow = deploys.find((x) => x.id === live)
  const replacedBy = d.replaced_by ? deploys.find((x) => x.id === d.replaced_by) : undefined
  const meta = (
    <span className="iii-cloud-desc">
      Deploy #{d.number} · <Mono>{sha7(d.commit.sha)}</Mono> · {whenOf(d, deploys, live)} · {deploySource(d)}
      {d.checked_ready ? '' : ', not checked here first'}
      {byYou ? ' · by you' : ''}
    </span>
  )
  const file = composeFile(project?.environment?.manifest)
  const fileLine = project?.repository ? (
    <span className="iii-cloud-desc">
      Current app file <Mono>{file}</Mono> ·{' '}
      <a
        className="iii-cloud-link"
        href={`https://github.com/${project.repository}/blob/${d.commit.sha}/${file}`}
        target="_blank"
        rel="noreferrer"
      >
        see the file in {sha7(d.commit.sha)}
      </a>
    </span>
  ) : null
  const foldedLog = narrow && failed && failureTail(d).lines.length > 0
  return (
    <div className="iii-cloud-ddetail">
      {narrow ? (
        <div className="iii-cloud-row">
          <IconButton label="Back to deploys" variant="ghost" className="iii-cloud-touch" onClick={() => open()}>
            <ArrowLeft />
          </IconButton>
          <span className="iii-cloud-dhead">
            {env} · deploy #{d.number}
          </span>
          <Word d={d} live={live} />
        </div>
      ) : null}
      {failed ? (
        <>
          {narrow ? null : (
            <div className="iii-cloud-dmeta">
              {meta}
              {fileLine}
            </div>
          )}
          <FailureCard
            host={host}
            status={status}
            deploy={d}
            serving={earlierLive}
            narrow={narrow}
            log={narrow}
            meta={narrow ? meta : undefined}
          />
          {status.agent && (status.agent.kind ?? 'deploy_fix') === 'deploy_fix' ? (
            <AgentRow host={host} agent={status.agent} branch={status.project?.workspace?.branch} />
          ) : null}
        </>
      ) : (
        <div className="iii-cloud-dmeta">
          {meta}
          {narrow ? null : fileLine}
          <h2 className="iii-cloud-h">{headline(d, live)}</h2>
        </div>
      )}
      {d.replaced_by ? (
        <Notice
          variant="info"
          headline={`Deploy ${replacedBy ? `#${replacedBy.number}` : sha7(d.replaced_by)} replaced it before it went live`}
          action={
            <Button size="sm" variant="secondary" onClick={() => open(d.replaced_by as string)}>
              Open that deploy
            </Button>
          }
        />
      ) : null}
      {needsAction(d) ? (
        <Notice
          variant="warn"
          headline="Needs your action"
          detail="iii cloud holds this deploy until you approve the resources and settings it would change."
          action={
            <Button size="sm" variant="secondary" onClick={reviewWaiting}>
              Review it
            </Button>
          }
        />
      ) : null}
      {d.outcome === 'ready' && live && d.id !== live ? (
        <Rollback host={host} status={status} envId={envId} target={d} from={servingNow} list={list} open={open} />
      ) : null}
      <Stages d={d} />
      {foldedLog ? null : <LogPanel host={host} status={status} envId={envId} d={d} />}
    </div>
  )
}

function Stages({ d }: { d: Deploy }) {
  const rows = deployRows(d)
  if (rows.length === 0) return null
  const failed = rows.find((r) => r.tone === 'alert')
  const now = rows.find((r) => r.tone === 'live')
  const summary = [
    `${rows.filter((r) => r.tone === 'ok').length} of ${rows.length} passed`,
    failed ? `${failed.label} failed` : null,
    now ? `${now.label} running` : null,
  ]
    .filter(Boolean)
    .join(' · ')
  return (
    <CollapsibleCard defaultOpen={d.outcome === 'running'}>
      <CollapsibleCardTrigger className="iii-cloud-fold">
        <span className="iii-cloud-fold-title">Stages</span>
        <span className="iii-cloud-faint">{summary}</span>
      </CollapsibleCardTrigger>
      <CollapsibleCardContent>
        <List aria-label="Stages" className="iii-cloud-fold-body">
          {rows.map((r) => (
            <ListItem
              as="div"
              key={r.key}
              leading={<Dot tone={r.tone} />}
              label={
                <span className={r.tone === 'edge' ? 'iii-cloud-faint' : undefined}>
                  {r.label}
                  {r.detail ? `: ${r.detail}` : null}
                </span>
              }
              trailing={<span className="iii-cloud-mono iii-cloud-faint">{r.time}</span>}
            />
          ))}
        </List>
      </CollapsibleCardContent>
    </CollapsibleCard>
  )
}

type Source = 'worker' | 'release' | 'build'
const SOURCES: { value: Source; label: string; lines: string }[] = [
  { value: 'worker', label: 'Workers', lines: 'No worker lines' },
  { value: 'release', label: 'Release', lines: 'No release command lines' },
  { value: 'build', label: 'Build', lines: 'No build lines' },
]

/** The log that failed first: the build's, the release command's, else the workers'. */
function firstSource(d: Deploy): Source {
  if ((d.failure?.stage ?? d.error?.stage) === 'build') return 'build'
  return d.failure?.release_command || d.release_row?.status === 'failed' ? 'release' : 'worker'
}

/** The deploy's log from iii cloud (`GET /logs`), read again every 5 s while it runs: the SSE tail is not reachable. */
function LogPanel({ host, status, envId, d }: { host: Host; status: Status; envId: string; d: Deploy }) {
  const [source, setSource] = useState<Source>(() => firstSource(d))
  const [errorsOnly, setErrorsOnly] = useState(false)
  const running = d.outcome === 'running'
  const path = `/logs?deployment_id=${encodeURIComponent(d.id)}&source=${source}${errorsOnly ? '&level=error' : ''}`
  const logs = useCloud<LogPage>(host, status, envId, path, { pollMs: running ? 5000 : undefined })
  const lines = (logs.data?.lines ?? []).map((l) => l.message).reverse()
  // The worker kept the failing stage's last lines: they stand in when iii cloud has none to give.
  const kept = !errorsOnly && source === firstSource(d) ? failureTail(d).lines : []
  const none = SOURCES.find((s) => s.value === source)?.lines
  let body: ReactNode
  if (lines.length) body = <LogLines lines={lines} />
  else if (logs.loading && !logs.data && !logs.error) body = <Skeleton className="iii-cloud-skeleton" />
  else
    body = (
      <>
        {logs.error ? (
          <ErrorPanel
            error={logs.error}
            action={
              <Button size="sm" variant="secondary" onClick={logs.refetch}>
                Try again
              </Button>
            }
          />
        ) : null}
        {kept.length ? (
          <LogLines lines={kept} />
        ) : logs.error ? null : (
          <EmptyState
            compact
            title={`${none} for this deploy`}
            description={
              errorsOnly
                ? 'Nothing at error level. Turn off Errors only to see every line.'
                : running
                  ? 'They show here as the deploy writes them.'
                  : 'iii cloud keeps logs for 7 days.'
            }
          />
        )}
      </>
    )
  return (
    <Section
      title="Log"
      end={running ? 'follows the deploy, read every 5 s' : d.finished_at ? `ended ${clock(d.finished_at)}` : undefined}
    >
      <div className="iii-cloud-logbody">
        <div className="iii-cloud-row">
          <SegmentedControl
            variant="tabs"
            aria-label="Logs"
            value={source}
            onChange={setSource}
            options={SOURCES.map((s) => ({ value: s.value, label: s.label, icon: false }))}
          />
          <span className="iii-cloud-grow" />
          <Checkbox label="Errors only" checked={errorsOnly} onChange={(e) => setErrorsOnly(e.target.checked)} />
        </div>
        {body}
      </div>
    </Section>
  )
}

const asError = (err: unknown): WorkerError =>
  workerError(err, (err as { code?: string }).code ?? errorCode(err), errorMessage(err))

const OWNERS = /^(owner|admin)$/i

/**
 * Roll back to an earlier live deploy (Mng7): iii cloud's preview, then the person confirms (production: by
 * typing its name). The deployments are read again before the rollback; a newer one locks it.
 */
function Rollback({
  host,
  status,
  envId,
  target,
  from,
  list,
  open,
}: {
  host: Host
  status: Status
  envId: string
  target: Deploy
  /** What serves now. */
  from: Deploy | undefined
  list: DeploymentList | null
  open: (id?: string) => void
}) {
  const [shown, setShown] = useState<{ seen: string | null; at: string } | null>(null)
  const [typed, setTyped] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<WorkerError | null>(null)
  const [changed, setChanged] = useState<DeploymentList['deployments'][number] | null>(null)
  const stale = useLocked(host, status)
  const preview = useCloud<Preview>(host, status, envId, shown ? `/deployments/${target.id}/rollback` : null)
  const env = status.project?.environment
  const name = env?.name ?? 'the environment'
  const label = `Roll back ${name} to #${target.number}`
  const newest = list?.deployments[0]
  const newestByYou = !!newest?.triggered_by.user_id && newest.triggered_by.user_id === status.connection.user?.id
  const role = roleOf(status)
  const begin = () => {
    setShown({
      seen: newest?.id ?? status.deploys.find((d) => d.environment_id === envId)?.id ?? null,
      at: new Date().toISOString(),
    })
    setTyped('')
    setError(null)
    setChanged(null)
  }
  const confirm = async () => {
    if (!shown) return
    setBusy(true)
    setError(null)
    try {
      const now = await cloudRead<DeploymentList>(host, status, envId, '/deployments')
      const top = now.deployments[0]
      if (top && top.id !== shown.seen) {
        setChanged(top)
        return
      }
      const made = await cloudWrite<{ id?: string }>(host, status, envId, 'POST', `/deployments/${target.id}/rollback`)
      setShown(null)
      await hubOf(host).refresh(true)
      if (made?.id) open(made.id)
    } catch (err) {
      setError(asError(err))
    } finally {
      setBusy(false)
    }
  }
  const openNewer = async (id: string) => {
    setShown(null)
    await hubOf(host).refresh(true)
    open(id)
  }
  const p = preview.data
  const row = (k: string, v: ReactNode) => (
    <div className="iii-cloud-rb-row" key={k}>
      <dt>{k}</dt>
      <dd>{v}</dd>
    </div>
  )
  return (
    <>
      <span>
        <Button variant="secondary" disabled={stale} onClick={begin}>
          {label}
        </Button>
      </span>
      <Dialog open={!!shown} onOpenChange={(o) => (o ? undefined : setShown(null))}>
        <DialogContent>
          <DialogTitle>{label}?</DialogTitle>
          <DialogDescription>
            iii cloud's preview, checked against what {name} serves at {shown ? clock(shown.at) : ''}.
          </DialogDescription>
          {preview.error ? (
            <ErrorPanel error={preview.error} />
          ) : !p ? (
            <Skeleton className="iii-cloud-skeleton" />
          ) : (
            <dl className="iii-cloud-rb">
              {row(
                'Code',
                <>
                  {name} goes back to <Mono>{sha7(target.commit.sha)}</Mono>
                  {from ? (
                    <>
                      {' '}
                      from <Mono>{sha7(from.commit.sha)}</Mono>
                    </>
                  ) : null}
                  . Your branch keeps the newer commits, so the next deploy brings them back.
                </>,
              )}
              {p.returns.length ? row('Goes back', p.returns.join(', ')) : null}
              {p.stays.length ? row('Stays', p.stays.join(', ')) : null}
              {p.side_effects.length ? row('Side effects', p.side_effects.join(' ')) : null}
              {p.queue ? row('Queue', p.queue) : null}
              {newest
                ? row(
                    'Newer deploys',
                    `#${newest.number} is the newest${newestByYou ? ', by you' : ''}. If anyone deploys before you confirm, this stops and shows what changed.`,
                  )
                : null}
            </dl>
          )}
          {changed ? (
            <Notice
              variant="warn"
              headline={`Deploy #${changed.number} started on ${name} at ${clock(changed.created_at)}`}
              detail="Rolling back now would undo it. Look at that deploy first."
              action={
                <Button size="sm" variant="secondary" onClick={() => void openNewer(changed.id)}>
                  Open #{changed.number}
                </Button>
              }
            />
          ) : null}
          {env?.production ? (
            <div className="iii-cloud-rb-type">
              <span className="iii-cloud-lbl">
                Type {name} to roll back to #{target.number}
              </span>
              <Input
                aria-label={`Type ${name} to confirm`}
                placeholder={`Type ${name}`}
                value={typed}
                onChange={setTyped}
              />
            </div>
          ) : null}
          {error ? <ErrorPanel error={error} /> : null}
          <div className="iii-cloud-rb-foot">
            <span className="iii-cloud-desc iii-cloud-grow">
              {env?.production
                ? `This serves real users. Owners and Admins only${role && !OWNERS.test(role) ? `; you are ${role} on this team` : ''}.`
                : 'For production you would also type its name, and only Owners and Admins can.'}
            </span>
            <DialogClose asChild>
              <Button variant="secondary">Cancel</Button>
            </DialogClose>
            <Button
              variant="primary"
              disabled={!p || busy || stale || !!changed || (!!env?.production && typed !== name)}
              onClick={() => void confirm()}
            >
              {busy ? 'Rolling back…' : label}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  )
}

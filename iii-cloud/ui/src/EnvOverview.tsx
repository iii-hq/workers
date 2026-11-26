import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  Chip,
  ConfirmDialog,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Input,
  Panel,
  PanelBody,
  PanelHeader,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { Check } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { type Cloud, cloudWrite, useCloud, useDeployments } from './api'
import { clock } from './asked'
import { useLocked } from './hub'
import { composeFile, deployOrigin, plural, servingDeploy, sha7, sizeOf, type Tone, when } from './model'
import { Dot, ErrorPanel, Facts, Mono, Notice, SectionCard as Section, StatePill } from './parts'
import type { EnvTabProps, Route } from './route'
import type { CloudDeployment, DeploymentList, Members, Status, WorkerError } from './types'

// ---- iii cloud's answers the Manage pages read (cloud-v1.yaml), only the fields they use ----

export { Feeds, useDeployments } from './api'
export { deployState, type Live, liveOf, when } from './model'
// The deployment types, the one-source functions and the Feeds context moved to the foundation; re-exported
// so the Manage pages keep importing them from here.
export type { CloudDeployment, DeploymentList, Members } from './types'
export type Health = {
  status: 'healthy' | 'degraded' | 'down' | 'stopped'
  checked_at: string
  workers: { worker: string; ready: boolean }[]
}
type Group = {
  name: string
  current_workers?: string[] | null
  size: string
  effective_size?: string | null
  replicas: { current: number }
  operation?: { state: 'queued' | 'applying' | 'ready' | 'failed'; requested_size: string } | null
  waiting_for_machine?: boolean
  pending?: boolean
}
export type Machines = { status: 'running' | 'stopped'; groups: Group[] }
type Domain = {
  id: string
  hostname: string
  status: 'pending_dns' | 'verifying' | 'active' | 'failed'
  verification: { type: string; name: string; value: string }
}

// ---- shared with Manage.tsx (and any env tab) ----

/** The shared pieces (parts.tsx) under the names the Manage pages already use. */
export const StateChip = ({ live }: { live: { label: string; tone: Tone } }) => <StatePill state={live} />
export { Section }

const sameCommit = (a: string, b: string) => !!a && !!b && (a.startsWith(b) || b.startsWith(a))

/**
 * The selected environment's check passed on a commit it doesn't serve, and nothing is deploying:
 * the person reviews and deploys it. `home` words it for the project's home.
 */
export function WaitingPanel({
  status,
  envId,
  feed,
  go,
  locked,
  home,
}: {
  status: Status
  envId: string
  feed: Cloud<DeploymentList>
  go: (r: Route) => void
  locked: boolean
  home?: boolean
}) {
  const c = status.check
  const env = status.project?.environments.find((e) => e.id === envId)
  if (!c?.ready || !env || c.environment_id !== envId || (feed.loading && !feed.data)) return null
  const list = feed.data?.deployments ?? []
  const running =
    list.some((d) => d.status === 'queued' || d.status === 'in_progress') ||
    status.deploys.some((d) => d.environment_id === envId && d.outcome === 'running')
  const live = (list.find((d) => d.current)?.commit.sha ?? servingDeploy(status, envId)?.commit.sha) || null
  if (running || (live && sameCommit(live, c.commit))) return null
  const commits = c.commits ?? []
  const top = commits.find((x) => sameCommit(x.sha, c.commit)) ?? commits[0]
  const here = c.run?.status === 'passed' ? ' on iii cloud and here' : ''
  const at = clock(c.checked_at)
  const subject = top ? (
    <>
      <Mono>{sha7(top.sha)}</Mono> "{top.message}"
    </>
  ) : (
    <Mono>{sha7(c.commit)}</Mono>
  )
  const detail = home ? (
    <>
      {commits.length ? (
        <>
          It has {plural(commits.length, 'commit')} {env.name} doesn't serve, {subject}.{' '}
        </>
      ) : null}
      The check passed{here} at {at}.
    </>
  ) : (
    <>
      {subject} · checked{here} at {at} ·{' '}
      {live ? (
        <>
          {env.name} still serves <Mono>{sha7(live)}</Mono>.
        </>
      ) : (
        `nothing is live on ${env.name} yet.`
      )}
    </>
  )
  return (
    <Notice
      variant="warn"
      headline={
        home
          ? `Waiting for you: ${env.name} can get ${sha7(c.commit)}`
          : `Waiting for you: ${commits.length ? plural(commits.length, 'commit') : sha7(c.commit)} passed the check`
      }
      detail={detail}
      action={
        <Button variant="primary" disabled={locked} onClick={() => go({ at: 'deploy', env: envId })}>
          Review and deploy
        </Button>
      }
    />
  )
}

/** "11 workers in 3 groups": the workers the serving deployment runs, resource workers counted once. */
export function workerCount(m: Machines | null): { workers: number; groups: number } | null {
  const serving = m?.groups.filter((g) => g.current_workers) ?? []
  const workers = new Set(serving.flatMap((g) => g.current_workers ?? [])).size
  return workers ? { workers, groups: serving.length } : null
}

/** "11 of 11 workers up", or that it is stopped, with the dot it gets. */
export function healthOf(h: Health, envName: string): { text: string; tone: Tone } {
  if (h.status === 'stopped') return { text: `${envName} is stopped`, tone: 'ghost' }
  const up = h.workers.filter((w) => w.ready).length
  return {
    text: `${up} of ${h.workers.length} workers up`,
    tone: h.status === 'healthy' ? 'ok' : h.status === 'degraded' ? 'warn' : 'alert',
  }
}

// ---- Mng6: the Overview tab ----

/** What a group is doing; how much memory it uses and since when it runs are not in iii cloud's answer. */
function groupState(g: Group, stopped: boolean): string {
  if (stopped) return 'stopped'
  if (g.pending) return 'starts with the next deploy'
  if (g.waiting_for_machine) return 'waiting for a machine'
  const op = g.operation
  if (op?.state === 'queued' || op?.state === 'applying') return `resizing to ${sizeOf(op.requested_size)}`
  if (op?.state === 'failed') return 'resize failed'
  return 'running'
}

const DOMAIN_STATE: Record<Domain['status'], string> = {
  pending_dns: 'waiting for its TXT record',
  verifying: 'verifying',
  active: 'active',
  failed: 'failed',
}

/** One POST to iii cloud as the person, with the cloud's answer (a 403 names the role) as a plain error. */
function useWrite(host: Host, status: Status, envId: string) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<WorkerError | null>(null)
  /** `{body}` once iii cloud took it (a 202 has no body), null when it refused. */
  const run = async <T,>(path: string, body?: unknown): Promise<{ body: T } | null> => {
    setBusy(true)
    setError(null)
    try {
      return { body: await cloudWrite<T>(host, status, envId, 'POST', path, body) }
    } catch (err) {
      const e = err as { code?: string; message?: string; status?: number }
      setError({ code: e.code ?? 'ERROR', message: e.message ?? '', status: e.status })
      return null
    } finally {
      setBusy(false)
    }
  }
  return { busy, error, setError, run }
}

type Ask = { kind: 'restart'; group: string } | { kind: 'stop' } | null

/** An environment's Overview tab (Mng6): what it serves, its health, machines and address. */
export function EnvOverview({ host, status, go, envId }: EnvTabProps) {
  const p = status.project
  const env = p?.environment?.id === envId ? p.environment : null
  const name = env?.name ?? ''
  const feed = useDeployments(host, status, envId)
  const current = feed.data?.deployments.find((d) => d.current) ?? null
  const other = !!current?.triggered_by.user_id && current.triggered_by.user_id !== status.connection.user?.id
  const members = useCloud<Members>(host, status, null, other ? '/members' : null)
  const health = useCloud<Health>(host, status, envId, current ? '/health' : null)
  const machines = useCloud<Machines>(host, status, envId, '/machines')
  const domains = useCloud<{ domains: Domain[] }>(host, status, envId, '/domains')
  const locked = useLocked(host, status)
  const write = useWrite(host, status, envId)
  const [ask, setAsk] = useState<Ask>(null)
  const [done, setDone] = useState<string | null>(null)
  if (!env) return null
  // The deployments list opens on the deployment that serves; the secrets page is its own route.
  const tab = (t: 'deploys' | 'settings') =>
    go(t === 'settings' ? { at: 'secrets', env: envId } : { at: 'env', env: envId, deploy: current?.id })
  const origin = current ? deployOrigin(status, current, current, members.data) : null
  const count = workerCount(machines.data)
  const stopped = machines.data?.status === 'stopped'
  const groups = machines.data?.groups ?? []
  const h = health.data ? healthOf(health.data, name) : null

  const confirm = async () => {
    if (!ask) return
    const out =
      ask.kind === 'restart'
        ? await write.run(`/machines/groups/${encodeURIComponent(ask.group)}/restart`)
        : await write.run('/machines/stop')
    setAsk(null)
    if (out === null) return
    setDone(ask.kind === 'restart' ? `${ask.group} is restarting` : `${name} is stopping`)
    machines.refetch()
    health.refetch()
  }

  return (
    <>
      <WaitingPanel status={status} envId={envId} feed={feed} go={go} locked={locked} />
      <div className="iii-cloud-mng-grid">
        <Section title="Serving">
          {feed.error ? (
            <ErrorPanel error={feed.error} />
          ) : !feed.data ? (
            <Skeleton className="iii-cloud-skeleton-row" />
          ) : current ? (
            <>
              <Facts
                rows={[
                  [
                    'Commit',
                    <>
                      <Mono>{sha7(current.commit.sha)}</Mono> {current.commit.message ?? ''}
                    </>,
                  ],
                  [
                    'Deployment',
                    `#${current.number} · live since ${when(current.finished_at ?? current.created_at)} · ${origin?.origin}`,
                  ],
                  [
                    'Compose file',
                    <>
                      <Mono>{composeFile(env.manifest)}</Mono>
                      {count ? ` · ${plural(count.workers, 'worker')} in ${plural(count.groups, 'group')}` : ''} ·{' '}
                      <button type="button" className="iii-cloud-linkbtn" onClick={() => tab('settings')}>
                        Change…
                      </button>
                    </>,
                  ],
                  ['Checked', origin?.checked ? 'before the deployment' : 'not checked first'],
                ]}
              />
              <span>
                <button type="button" className="iii-cloud-linkbtn iii-cloud-small" onClick={() => tab('deploys')}>
                  All deployments
                </button>
              </span>
            </>
          ) : (
            <p className="iii-cloud-desc">Nothing is live on {name} yet. Deploy from this machine to start it.</p>
          )}
        </Section>
        <Section title="Health">
          {!current ? (
            <p className="iii-cloud-desc">Nothing runs on {name} yet.</p>
          ) : health.error ? (
            <ErrorPanel error={health.error} />
          ) : h && health.data ? (
            <div className="iii-cloud-health">
              <span className="iii-cloud-dottext">
                {h.tone === 'ok' ? <Check aria-hidden /> : <Dot tone={h.tone} />}
                iii cloud: {h.text}
              </span>
              <span className="iii-cloud-desc">checked {clock(health.data.checked_at)}</span>
            </div>
          ) : (
            <Skeleton className="iii-cloud-skeleton-row" />
          )}
        </Section>
        <Section title="Machines" flush>
          {machines.error ? (
            <PanelBody className="iii-cloud-panel-body">
              <ErrorPanel error={machines.error} />
            </PanelBody>
          ) : !machines.data ? (
            <PanelBody className="iii-cloud-panel-body">
              <Skeleton className="iii-cloud-skeleton-row" />
            </PanelBody>
          ) : (
            <TableViewport>
              <Table inset className="iii-cloud-table">
                <TableHeader>
                  <TableRow>
                    <TableHead>Group</TableHead>
                    <TableHead>Machines</TableHead>
                    <TableHead>State</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {groups.map((g) => (
                    <TableRow key={g.name}>
                      <TableCell>
                        <Mono>{g.name}</Mono>
                      </TableCell>
                      <TableCell>
                        {g.replicas.current} × {sizeOf(g.effective_size ?? g.size)}
                      </TableCell>
                      <TableCell>{groupState(g, stopped)}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </TableViewport>
          )}
          <PanelBody className="iii-cloud-panel-body">
            {write.error ? <ErrorPanel error={write.error} /> : null}
            {done ? (
              <Notice
                variant="success"
                headline={done}
                detail="iii cloud replaces the machines; it takes a minute."
                action={
                  <Button variant="secondary" size="sm" onClick={() => setDone(null)}>
                    Dismiss
                  </Button>
                }
              />
            ) : null}
            <div className="iii-cloud-row">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="secondary" size="sm" disabled={locked || stopped || write.busy || !groups.length}>
                    Restart a group…
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start">
                  {groups.map((g) => (
                    <DropdownMenuItem key={g.name} onSelect={() => setAsk({ kind: 'restart', group: g.name })}>
                      {g.name}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
              <Button
                variant="secondary"
                size="sm"
                disabled={locked || stopped || write.busy || !machines.data}
                onClick={() => setAsk({ kind: 'stop' })}
              >
                Stop {name}
              </Button>
            </div>
          </PanelBody>
        </Section>
        <Address host={host} status={status} envId={envId} domains={domains} locked={locked} />
      </div>
      <ConfirmDialog
        open={ask !== null}
        onOpenChange={(open) => {
          if (!open) setAsk(null)
        }}
        title={ask?.kind === 'restart' ? `Restart ${ask.group} on ${name}?` : `Stop ${name}?`}
        description={
          ask?.kind === 'restart'
            ? `iii cloud replaces ${ask.group}'s machines. Its workers start again and read their variables again.`
            : `Every group and the engine stop, so ${name} answers nothing until it is started again. Its resources and data stay.`
        }
        confirmLabel={ask?.kind === 'restart' ? `Restart ${ask.group}` : `Stop ${name}`}
        tone={ask?.kind === 'stop' ? 'danger' : 'default'}
        onConfirm={confirm}
      />
    </>
  )
}

function Address({
  host,
  status,
  envId,
  domains,
  locked,
}: {
  host: Host
  status: Status
  envId: string
  domains: Cloud<{ domains: Domain[] }>
  locked: boolean
}) {
  const env = status.project?.environment
  const write = useWrite(host, status, envId)
  const [open, setOpen] = useState(false)
  const [hostname, setHostname] = useState('')
  const [added, setAdded] = useState<Domain | null>(null)
  const list = domains.data?.domains ?? []
  const close = () => {
    setOpen(false)
    setHostname('')
    setAdded(null)
    write.setError(null)
  }
  const add = async () => {
    const out = await write.run<Domain>('/domains', { hostname: hostname.trim() })
    if (!out) return
    setAdded(out.body)
    domains.refetch()
  }
  return (
    <Section title="Address">
      <Facts
        rows={[
          ['Default', env?.url ? <Mono>{env.url.replace(/^https?:\/\//, '')}</Mono> : 'none yet'],
          [
            'Your domains',
            domains.error ? (
              <ErrorPanel error={domains.error} />
            ) : !domains.data ? (
              <Skeleton className="iii-cloud-skeleton-row" />
            ) : list.length ? (
              <ul className="iii-cloud-plain">
                {list.map((d) => (
                  <li key={d.id}>
                    <Mono>{d.hostname}</Mono> · {DOMAIN_STATE[d.status]}
                    {d.status === 'pending_dns' ? (
                      <>
                        {' '}
                        · TXT <Mono>{d.verification.name}</Mono> <Mono>{d.verification.value}</Mono>
                      </>
                    ) : null}
                  </li>
                ))}
              </ul>
            ) : (
              'none'
            ),
          ],
        ]}
      />
      <span>
        <Button variant="secondary" size="sm" disabled={locked} onClick={() => setOpen(true)}>
          Add a domain
        </Button>
      </span>
      <Dialog open={open} onOpenChange={(o) => (o ? setOpen(true) : close())}>
        <DialogContent>
          <DialogTitle>Add a domain to {env?.name}</DialogTitle>
          <DialogDescription>
            iii cloud checks a TXT record you add at your DNS provider, then gets a certificate for it.
          </DialogDescription>
          {added ? (
            <div className="iii-cloud-form">
              <p className="iii-cloud-desc">Add this TXT record for {added.hostname}, then iii cloud verifies it:</p>
              <Facts
                rows={[
                  ['Name', <Mono>{added.verification.name}</Mono>],
                  ['Value', <Mono>{added.verification.value}</Mono>],
                ]}
              />
              <div className="iii-cloud-dlg-foot">
                <Button variant="primary" onClick={close}>
                  Done
                </Button>
              </div>
            </div>
          ) : (
            <form
              className="iii-cloud-form"
              onSubmit={(e) => {
                e.preventDefault()
                if (hostname.trim()) void add()
              }}
            >
              <label htmlFor="iii-cloud-domain" className="iii-cloud-lbl">
                Domain
              </label>
              <Input id="iii-cloud-domain" value={hostname} onChange={setHostname} placeholder="api.example.com" />
              {write.error ? <ErrorPanel error={write.error} /> : null}
              <div className="iii-cloud-dlg-foot">
                <Button variant="secondary" onClick={close}>
                  Cancel
                </Button>
                <Button variant="primary" type="submit" disabled={!hostname.trim() || write.busy}>
                  Add domain
                </Button>
              </div>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </Section>
  )
}

import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  Chip,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  EmptyState,
  IconButton,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  PageMain,
  PageSidebar,
  PanelBody,
  SegmentedControl,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { usePaneState } from '@iii-dev/console-ui/hooks'
import { ChevronRight, Cloud, Ellipsis, Plus, ShieldCheck } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { type Cloud as CloudRead, useCloud } from './api'
import { clock } from './asked'
import { EnvChecks } from './EnvChecks'
import { EnvDeploys } from './EnvDeploys'
import {
  byWho,
  type CloudDeployment,
  type DeploymentList,
  deployState,
  EnvOverview,
  Feeds,
  type Health,
  healthOf,
  type Live,
  liveOf,
  type Machines,
  type Members,
  Section,
  StateChip,
  WaitingPanel,
  when,
  workerCount,
} from './EnvOverview'
import { EnvSettings } from './EnvSettings'
import { useActions, useLocked, useStale } from './hub'
import {
  composeFile,
  isDirectEnv,
  needsAction,
  plural,
  projectName,
  roleOf,
  sha7,
  signInRequired,
  tilde,
} from './model'
import { NewEnv } from './NewEnv'
import { Dot, ErrorPanel, Facts, Fresh, Mono, Notice, PathBar, StaleNotice, useLeave, useSwitch } from './parts'
import { type AreaProps, ENV_TABS, type EnvTab, envState, pathOf, type RouteAt } from './route'
import type { Status, WorkerError } from './types'

type ManageProps = AreaProps<RouteAt<'home' | 'env' | 'new-env'>>
type Env = NonNullable<Status['project']>['environments'][number]
type Feed = CloudRead<DeploymentList>
type FeedMap = Record<string, Feed>

const TAB_LABEL: Record<EnvTab, string> = {
  overview: 'Overview',
  deploys: 'Deploys',
  settings: 'Settings',
  checks: 'Checks',
}

const pending: Feed = { data: null, error: null, loading: true, refetch: () => undefined }

/**
 * Reads one environment's `/deployments` and hands it up: Manage reads each environment once per
 * refresh (the hub's, never faster) and every part of the page shares it.
 */
function FeedReader({
  host,
  status,
  envId,
  onRead,
}: {
  host: Host
  status: Status
  envId: string
  onRead: (id: string, c: Feed) => void
}) {
  const c = useCloud<DeploymentList>(host, status, envId, '/deployments')
  // `c` is a new object every render; what it says is data, error and loading.
  useEffect(() => onRead(envId, c), [envId, c.data, c.error, c.loading, onRead])
  return null
}

/** The project: its environments in the sidebar, Home, one page per environment (Mng1, Mng5, Mng6, Mng9). */
export function ManageArea(props: ManageProps) {
  const { host, status, route, narrow } = props
  const { error, call } = useActions(host)
  const [feeds, setFeeds] = useState<FeedMap>({})
  const onRead = useCallback((id: string, c: Feed) => setFeeds((f) => ({ ...f, [id]: c })), [])
  const stale = useStale(host)
  const project = status.project
  if (!project) return null
  const deleted = status.problems?.find((p) => p.code === 'PROJECT_DELETED')
  const page = deleted ? (
    <div className="iii-cloud-body">
      <Deleted host={host} status={status} problem={deleted} />
    </div>
  ) : route.at === 'env' ? (
    <EnvPage {...props} route={route} feeds={feeds} />
  ) : route.at === 'new-env' ? (
    <NewEnv {...props} route={route} />
  ) : narrow ? (
    <PhoneHome {...props} route={route} feeds={feeds} />
  ) : (
    <Home {...props} route={route} feeds={feeds} />
  )
  const notes = [
    route.at === 'home' && route.note ? <Notice key="note" variant="info" headline={route.note} /> : null,
    stale ? <StaleNotice key="stale" host={host} status={status} /> : null,
    signInRequired(status) ? (
      <Notice
        key="signin"
        variant="alert"
        headline="Sign in to iii cloud again"
        detail="iii cloud no longer accepts your sign-in. Until you sign in, what it serves may be out of date and nothing here can deploy."
        action={
          <Button
            variant="primary"
            size="sm"
            onClick={() => void call('connect', 'iii-cloud::connect', { method: 'browser' })}
          >
            Sign in again
          </Button>
        }
      />
    ) : null,
    error ? <ErrorPanel key="error" error={error} /> : null,
  ].filter(Boolean)
  return (
    <Feeds.Provider value={feeds}>
      {deleted
        ? null
        : project.environments.map((e) => (
            <FeedReader key={e.id} host={host} status={status} envId={e.id} onRead={onRead} />
          ))}
      {narrow ? null : <Sidebar {...props} feeds={feeds} />}
      <PageMain className="iii-cloud-main" data-stale={stale ? '' : undefined}>
        {notes.length ? <div className="iii-cloud-body iii-cloud-notes">{notes}</div> : null}
        {page}
      </PageMain>
    </Feeds.Provider>
  )
}

/** The selected environment's open pull request, the only one the worker follows. */
const openPr = (s: Status, envId: string) =>
  s.project?.environment?.id === envId && s.pr?.state === 'OPEN' ? s.pr : null

/** "Live · 9c04e7f", "Nothing live yet · PR #170". */
function liveLine(live: Live | null, s: Status, envId: string): string | undefined {
  if (!live) return undefined
  const pr = openPr(s, envId)
  return [live.label, live.sha ? sha7(live.sha) : null, pr ? `PR #${pr.number}` : null].filter(Boolean).join(' · ')
}

const ProductionIcon = () => <ShieldCheck role="img" aria-label="Production" size={16} />
const ProductionChip = () => (
  <Chip className="iii-cloud-prodchip">
    <ShieldCheck aria-hidden />
    Production
  </Chip>
)
/** Opening another environment selects it: something waiting on the person here asks first. */
function useEnvGo(host: Host, status: Status, go: ManageProps['go']) {
  const { leave, dialog } = useLeave(host, status)
  const toEnv = (e: Env, tab?: EnvTab) => {
    const open = () => go({ at: 'env', env: e.id, tab })
    return e.id === status.project?.environment?.id ? open() : leave(e.name, open)
  }
  return { toEnv, dialog }
}

function Sidebar({ host, status, route, go, feeds }: ManageProps & { feeds: FeedMap }) {
  const project = status.project!
  const name = projectName(status)
  const role = roleOf(status)
  const team = project.team?.name
  const envId = route.at === 'env' ? route.env : null
  const { toEnv, dialog } = useEnvGo(host, status, go)
  return (
    <PageSidebar label={name} narrowMode="inline" narrowBelow={640} defaultWidth={232}>
      <div className="iii-cloud-side">
        <List aria-label={`${name} environments`}>
          <ListItem
            leading={<Cloud />}
            label={name}
            description={`${project.repository}${team ? ` · team ${team}` : ''}`}
            selected={route.at === 'home'}
            aria-current={route.at === 'home' ? 'page' : undefined}
            onClick={() => go({ at: 'home' })}
          />
          <ListGroup>
            <ListGroupLabel>Environments</ListGroupLabel>
            {project.environments.map((e) => {
              const live = liveOf(feeds[e.id]?.data?.deployments)
              // The worker's own deploy going on says so first (iii cloud's list may not have it yet).
              const run = status.deploys.find(
                (d) => d.environment_id === e.id && d.outcome === 'running' && !needsAction(d),
              )
              return (
                <ListItem
                  key={e.id}
                  leading={<Dot tone={run ? 'live' : (live?.tone ?? 'ghost')} />}
                  label={e.name}
                  description={run ? `deploying #${run.number}` : liveLine(live, status, e.id)}
                  trailing={e.production ? <ProductionIcon /> : undefined}
                  selected={envId === e.id}
                  aria-current={envId === e.id ? 'page' : undefined}
                  onClick={() => toEnv(e, route.at === 'env' ? route.tab : undefined)}
                />
              )
            })}
          </ListGroup>
          <ListItem
            leading={<Plus />}
            label="New environment"
            selected={route.at === 'new-env'}
            aria-current={route.at === 'new-env' ? 'page' : undefined}
            onClick={() => go({ at: 'new-env', branch: project.checkout.branch ?? undefined })}
          />
        </List>
        <div className="iii-cloud-side-foot">
          <span className="iii-cloud-psmall" title={project.path}>
            Folder {tilde(project.path)}
          </span>
          <span className="iii-cloud-psmall">
            {status.connection.api_key
              ? `API key ${status.connection.api_key.secret}`
              : `Signed in${role ? ` · ${role}` : ''}${team ? ` on team ${team}` : ''}`}
          </span>
        </div>
      </div>
      {dialog}
    </PageSidebar>
  )
}

/** Every environment's newest deployments, newest first. */
function recent(feeds: FeedMap, envs: Env[], n: number): CloudDeployment[] {
  return envs
    .flatMap((e) => feeds[e.id]?.data?.deployments ?? [])
    .sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at))
    .slice(0, n)
}

/** "14:34 · you · checked here": "checked here" only for a deploy this worker made after a ready check. */
function deployWhen(d: CloudDeployment, s: Status, members: Members | null): string {
  const checked = s.deploys.some((x) => x.id === d.id && x.checked_ready)
  return [when(d.created_at), byWho(d, s, members), checked ? 'checked here' : null].filter(Boolean).join(' · ')
}

const article = (word: string) => (/^[aeiou]/i.test(word) ? `an ${word}` : `a ${word}`)

const AGENT_LINE: Record<string, string> = {
  starting: 'starting',
  working: 'working',
  waiting_for_you: 'waiting for you',
  waiting: 'waiting',
  stalled: 'quiet for a while',
}
/** "idle · last check passed 14:39", or what the agent is doing now. */
function agentLine(s: Status): string {
  const a = s.agent
  if (a && AGENT_LINE[a.state]) {
    const line = a.state === 'waiting' && a.waiting_for ? `waiting for ${a.waiting_for}` : AGENT_LINE[a.state]
    return [line, a.note].filter(Boolean).join(' · ')
  }
  const c = s.check
  return c ? `idle · last check ${c.ready ? 'passed' : 'found problems'} ${when(c.checked_at)}` : 'idle'
}

// ---- Mng1: the project's home ----

function Home({ host, status, route, go, feeds }: AreaProps<RouteAt<'home'>> & { feeds: FeedMap }) {
  const p = status.project!
  const envs = p.environments
  const name = projectName(status)
  const sel = p.environment
  const locked = useLocked(host, status)
  const members = useCloud<Members>(host, status, null, envs.length ? '/members' : null)
  const list = recent(feeds, envs, 6)
  return (
    <>
      <PathBar
        path={pathOf(route, status)}
        host={host}
        status={status}
        route={route}
        go={go}
        rule
        end={<Fresh host={host} status={status} />}
      >
        <span className="iii-cloud-desc">{plural(envs.length, 'environment')}</span>
      </PathBar>
      <div className="iii-cloud-body iii-cloud-mng iii-cloud-scroll">
        {envs.length === 0 ? (
          <EmptyState
            icon={Cloud}
            title={`${name} has no environments yet`}
            description="Set one up: the worker reads this folder and checks it before the first deploy."
            action={{ label: 'Set up an environment', onClick: () => go({ at: 'setup', step: 'target' }) }}
          />
        ) : (
          <>
            {sel ? (
              <WaitingPanel
                status={status}
                envId={sel.id}
                feed={feeds[sel.id] ?? pending}
                go={go}
                locked={locked}
                home
              />
            ) : null}
            <div className="iii-cloud-mng-grid">
              {envs.map((e) => (
                <EnvCard
                  key={e.id}
                  host={host}
                  status={status}
                  env={e}
                  feed={feeds[e.id]}
                  members={members.data}
                  go={go}
                />
              ))}
            </div>
            <Section
              title="Recent deploys, all environments"
              flush
              end={
                sel ? (
                  <button
                    type="button"
                    className="iii-cloud-linkbtn"
                    onClick={() => go({ at: 'env', env: sel.id, tab: 'deploys' })}
                  >
                    All deploys of {sel.name}
                  </button>
                ) : null
              }
            >
              {list.length ? (
                <TableViewport>
                  <Table inset className="iii-cloud-table">
                    <TableHeader>
                      <TableRow>
                        <TableHead>Deploy</TableHead>
                        <TableHead>Env</TableHead>
                        <TableHead>Commit</TableHead>
                        <TableHead>Result</TableHead>
                        <TableHead>When</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {list.map((d) => (
                        <TableRow key={d.id}>
                          <TableCell>#{d.number}</TableCell>
                          <TableCell>{envs.find((e) => e.id === d.environment_id)?.name}</TableCell>
                          <TableCell>
                            <Mono>{sha7(d.commit.sha)}</Mono> {d.commit.message ?? ''}
                          </TableCell>
                          <TableCell>
                            <StateChip live={deployState(d)} />
                          </TableCell>
                          <TableCell>{deployWhen(d, status, members.data)}</TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </TableViewport>
              ) : (
                <PanelBody className="iii-cloud-panel-body">
                  {envs.some((e) => !feeds[e.id]?.data && feeds[e.id]?.loading !== false) ? (
                    <Skeleton className="iii-cloud-skeleton-row" />
                  ) : (
                    <p className="iii-cloud-desc">No deploys yet.</p>
                  )}
                </PanelBody>
              )}
            </Section>
          </>
        )}
      </div>
    </>
  )
}

/** One environment on the home: what it serves, how it deploys, its app file, health and agent. */
function EnvCard({
  host,
  status,
  env,
  feed,
  members,
  go,
}: {
  host: Host
  status: Status
  env: Env
  feed: Feed | undefined
  members: Members | null
  go: ManageProps['go']
}) {
  const list = feed?.data?.deployments
  const live = liveOf(list)
  const current = list?.find((d) => d.current) ?? null
  const machines = useCloud<Machines>(host, status, env.id, current ? '/machines' : null)
  const health = useCloud<Health>(host, status, env.id, current ? '/health' : null)
  const p = status.project!
  const selected = p.environment?.id === env.id
  const pr = openPr(status, env.id)
  const role = roleOf(status)
  const direct = isDirectEnv(status, env)
  const count = workerCount(machines.data)
  const h = health.data ? healthOf(health.data, env.name) : null
  const rows: [string, React.ReactNode][] = [
    [
      'Serves',
      current ? (
        <>
          <Mono>{sha7(current.commit.sha)}</Mono> · deploy #{current.number} ·{' '}
          {when(current.finished_at ?? current.created_at)} by {byWho(current, status, members)}
        </>
      ) : feed?.error ? (
        '–'
      ) : list ? (
        'nothing yet'
      ) : (
        <Skeleton className="iii-cloud-skeleton-row" />
      ),
    ],
    [
      'Deploys',
      direct ? (
        <>
          from <Mono>{env.branch}</Mono>: here after the check, or on any push
        </>
      ) : (
        <>
          when a pull request into <Mono>{env.branch}</Mono> is merged
        </>
      ),
    ],
  ]
  if (pr) {
    const checked = status.check?.ready && status.check.commit === pr.head_sha
    rows.push([
      'Waiting on',
      <>
        <a href={pr.url} target="_blank" rel="noreferrer">
          pull request #{pr.number}
        </a>{' '}
        · {checked ? 'checked, not merged' : 'not merged'}
      </>,
    ])
  }
  if (current) {
    rows.push([
      'App file',
      <>
        <Mono>{composeFile(selected ? p.environment?.manifest : env.manifest)}</Mono>
        {count ? ` · ${plural(count.workers, 'worker')}` : ''}
      </>,
    ])
    if (h && health.data) rows.push(['Health', `${h.text} · checked ${clock(health.data.checked_at)}`])
  }
  if (selected) rows.push(['Your agent', agentLine(status)])
  if (env.production && role) {
    rows.push(['Who', `Owners and Admins ${direct ? 'deploy' : 'merge'} it; you're ${article(role)}`])
  }
  return (
    <Section
      title={env.name}
      onTitle={() => go({ at: 'env', env: env.id })}
      end={
        <>
          {env.production ? <ProductionChip /> : null}
          {live?.sha ? <StateChip live={live} /> : null}
        </>
      }
    >
      {feed?.error ? <ErrorPanel error={feed.error} /> : null}
      <Facts rows={rows} />
    </Section>
  )
}

/** A project's or environment's deletion: when it can still be restored, from iii cloud's deletions. */
function Deleted({ host, status, problem }: { host: Host; status: Status; problem: WorkerError }) {
  const p = status.project
  const project = problem.code === 'PROJECT_DELETED'
  const name = (project ? p?.cloud_project?.name : p?.environment?.name) ?? 'It'
  const list = useCloud<{
    deletions: {
      kind: string
      project?: { id: string } | null
      environment_id?: string | null
      restorable: boolean
      restorable_until: string | null
    }[]
  }>(host, status, null, '/deletions')
  const rec = list.data?.deletions.find((d) =>
    project
      ? d.kind === 'project' && d.project?.id === p?.cloud_project?.id
      : d.kind === 'environment' && d.environment_id === p?.environment?.id,
  )
  const until =
    rec?.restorable && rec.restorable_until
      ? `It can be restored until ${new Date(rec.restorable_until).toLocaleDateString([], { day: 'numeric', month: 'short' })} in the iii cloud console. `
      : ''
  const url = `${status.connection.cloud_url.replace(/\/$/, '')}/${p?.team?.slug ?? ''}`
  return (
    <Notice
      variant="alert"
      headline={`${name} was deleted in iii cloud`}
      detail={`${until}Nothing here can deploy to it.`}
      action={
        <Button asChild variant="secondary" size="sm">
          <a href={url} target="_blank" rel="noreferrer">
            Open in iii cloud
          </a>
        </Button>
      }
    />
  )
}

// ---- Mng5: the home on a phone ----

type Segment = 'envs' | 'deploys' | 'checks'

function PhoneHome(props: AreaProps<RouteAt<'home'>> & { feeds: FeedMap }) {
  const { host, status, route, go, feeds, paneKey } = props
  const [seg, setSeg] = usePaneState<Segment>(`iii-cloud:home:${paneKey}`, 'envs')
  const locked = useLocked(host, status)
  const members = useCloud<Members>(host, status, null, '/members')
  const p = status.project!
  const sel = p.environment
  const envs = p.environments
  const { toEnv, dialog } = useEnvGo(host, status, go)
  const rows = (list: CloudDeployment[]) => (
    <List aria-label="Deploys" className="iii-cloud-phonelist">
      {list.map((d) => (
        <ListItem
          key={d.id}
          label={`#${d.number}`}
          description={
            <>
              <Mono>{sha7(d.commit.sha)}</Mono> {d.commit.message ?? ''} · {deployWhen(d, status, members.data)}
            </>
          }
          trailing={
            <span className="iii-cloud-row">
              <StateChip live={deployState(d)} />
              <ChevronRight />
            </span>
          }
          onClick={() => go({ at: 'env', env: d.environment_id, tab: 'deploys', deploy: d.id })}
        />
      ))}
    </List>
  )
  return (
    <>
      <PathBar
        path={pathOf(route, status)}
        host={host}
        status={status}
        route={route}
        go={go}
        narrow
        rule
        end={<Fresh host={host} status={status} narrow />}
      />
      <SegmentedControl
        variant="tabs"
        aria-label="Sections"
        className="iii-cloud-seg"
        itemClassName="iii-cloud-seg-item"
        value={seg}
        onChange={setSeg}
        options={[
          { value: 'envs', label: 'Environments', icon: false },
          { value: 'deploys', label: 'Deploys', icon: false },
          { value: 'checks', label: 'Checks', icon: false },
        ]}
      />
      <div className="iii-cloud-body iii-cloud-mng iii-cloud-scroll">
        {seg === 'checks' ? (
          sel ? (
            <EnvChecks {...props} route={{ at: 'env', env: sel.id, tab: 'checks' }} envId={sel.id} />
          ) : (
            <EmptyState title="No environment chosen" description="Open an environment to see its checks." />
          )
        ) : seg === 'deploys' ? (
          <Section title="Recent deploys" flush>
            {rows(recent(feeds, envs, 6))}
          </Section>
        ) : envs.length === 0 ? (
          <EmptyState
            icon={Cloud}
            title={`${projectName(status)} has no environments yet`}
            description="Set one up: the worker reads this folder and checks it before the first deploy."
            action={{ label: 'Set up an environment', onClick: () => go({ at: 'setup', step: 'target' }) }}
          />
        ) : (
          <>
            {sel ? (
              <WaitingPanel
                status={status}
                envId={sel.id}
                feed={feeds[sel.id] ?? pending}
                go={go}
                locked={locked}
                home
              />
            ) : null}
            <Section title="Environments" flush>
              <List aria-label="Environments" className="iii-cloud-phonelist">
                {envs.map((e) => {
                  const list = feeds[e.id]?.data?.deployments
                  const live = liveOf(list)
                  const current = list?.find((d) => d.current)
                  const pr = openPr(status, e.id)
                  return (
                    <ListItem
                      key={e.id}
                      label={
                        <span className="iii-cloud-row">
                          {e.name}
                          {e.production ? <ProductionChip /> : null}
                        </span>
                      }
                      description={
                        pr
                          ? `Waiting on pull request #${pr.number}`
                          : current
                            ? `${sha7(current.commit.sha)} · deploy #${current.number} · ${when(current.finished_at ?? current.created_at)}`
                            : undefined
                      }
                      trailing={
                        <span className="iii-cloud-row">
                          {live ? <StateChip live={live} /> : null}
                          <ChevronRight />
                        </span>
                      }
                      onClick={() => toEnv(e)}
                    />
                  )
                })}
              </List>
            </Section>
            {sel ? (
              <Section title={`Recent on ${sel.name}`} flush>
                {rows((feeds[sel.id]?.data?.deployments ?? []).slice(0, 3))}
              </Section>
            ) : null}
          </>
        )}
      </div>
      {dialog}
    </>
  )
}

// ---- An environment: its header, tabs and the tab ----

/** A project on this machine that holds an environment: a recent one, or another cloud project of this folder. */
function holderOf(s: Status, envId: string) {
  const p = s.project
  const r = s.recent_projects?.find((x) => x.environment_id === envId)
  if (r) {
    return {
      key: `${r.path}:${r.cloud_project_id}`,
      name: r.cloud_project_name || r.repository,
      env: r.environment_name ?? 'That environment',
      payload: {
        path: r.path,
        cloud_project_id: r.cloud_project_id,
        environment_id: envId,
        mode: r.mode ?? 'worktree',
      },
    }
  }
  for (const c of p?.cloud_projects ?? []) {
    const e = c.environments?.find((x) => x.id === envId && !x.deleted)
    if (p && e && !c.deleted) {
      return {
        key: `${p.path}:${c.id}`,
        name: c.name,
        env: e.name,
        payload: { path: p.path, cloud_project_id: c.id, environment_id: envId, mode: p.workspace?.mode ?? 'worktree' },
      }
    }
  }
  return null
}

/** An environment the project in use doesn't have (a chat card of another project): where it is, and the way there. */
function ElsewhereEnv({ host, status, route, go }: AreaProps<RouteAt<'env'>>) {
  const { error, switchTo, dialog } = useSwitch(host, status, go)
  // Out of date, or iii cloud wants a fresh sign-in: no switch (the notes above say why).
  const locked = useLocked(host, status)
  const at = holderOf(status, route.env)
  return (
    <div className="iii-cloud-body">
      {error ? <ErrorPanel error={error} /> : null}
      {at ? (
        <EmptyState
          title={`${at.env} is in ${at.name}`}
          description="Switching moves this worker to that project; a running check, a deploy and your agent here keep going."
          action={
            locked
              ? undefined
              : { label: `Switch to ${at.name}`, onClick: () => switchTo(at.key, at.name, at.payload, at.env) }
          }
        />
      ) : (
        <EmptyState
          title="Not in a project on this machine"
          description="It may have been deleted, or it belongs to a project this machine hasn't used."
          action={{ label: 'Projects', onClick: () => go({ at: 'start' }) }}
        />
      )}
      {dialog}
    </div>
  )
}

function EnvPage(props: AreaProps<RouteAt<'env'>> & { feeds: FeedMap }) {
  const { host, status, route, go, narrow, feeds } = props
  const project = status.project
  const state = envState(status, route.env)
  const env = project?.environments.find((e) => e.id === route.env)
  const locked = useLocked(host, status)
  if (state === 'unknown' || !project || !env) return <ElsewhereEnv {...props} />
  const tab = route.tab ?? 'overview'
  const selected = state === 'selected'
  const live = liveOf(feeds[route.env]?.data?.deployments)
  const url = selected ? project.environment?.url?.replace(/^https?:\/\//, '') : null
  const gone = selected ? status.problems?.find((p) => p.code === 'ENVIRONMENT_DELETED') : undefined
  const toTab = (t: string) => go({ at: 'env', env: route.env, tab: t as EnvTab })
  const deploy = () => go({ at: 'deploy', env: route.env })
  const tabProps = { ...props, envId: route.env }
  const body = !selected ? (
    <Skeleton className="iii-cloud-skeleton" />
  ) : gone ? (
    <Deleted host={host} status={status} problem={gone} />
  ) : tab === 'deploys' ? (
    <EnvDeploys {...tabProps} />
  ) : tab === 'settings' ? (
    <EnvSettings {...tabProps} />
  ) : tab === 'checks' ? (
    <EnvChecks {...tabProps} />
  ) : (
    <EnvOverview {...tabProps} />
  )
  return (
    <>
      <PathBar
        path={pathOf(route, status)}
        host={host}
        status={status}
        route={route}
        go={go}
        narrow={narrow}
        end={
          <>
            <Fresh host={host} status={status} narrow={narrow} />
            {narrow ? (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <IconButton label="More actions" variant="ghost">
                    <Ellipsis />
                  </IconButton>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem disabled={locked} onSelect={deploy}>
                    Deploy from this machine
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            ) : (
              <Button size="sm" variant="secondary" disabled={locked} onClick={deploy}>
                Deploy from this machine
              </Button>
            )}
          </>
        }
      >
        {live ? <StateChip live={live} /> : null}
        {url && !narrow ? <span className="iii-cloud-mono iii-cloud-url">{url}</span> : null}
      </PathBar>
      {narrow ? (
        <>
          <SegmentedControl
            variant="tabs"
            aria-label={env.name}
            className="iii-cloud-seg"
            itemClassName="iii-cloud-seg-item"
            value={tab}
            onChange={toTab}
            options={ENV_TABS.map((t) => ({ value: t, label: TAB_LABEL[t], icon: false }))}
          />
          <div className="iii-cloud-body iii-cloud-mng iii-cloud-scroll">{body}</div>
        </>
      ) : (
        <Tabs value={tab} onValueChange={toTab} className="iii-cloud-tabs">
          <TabsList variant="line" aria-label={env.name}>
            {ENV_TABS.map((t) => (
              <TabsTrigger key={t} value={t} icon={false}>
                {TAB_LABEL[t]}
              </TabsTrigger>
            ))}
          </TabsList>
          <TabsContent value={tab} className="iii-cloud-body iii-cloud-mng iii-cloud-scroll">
            {body}
          </TabsContent>
        </Tabs>
      )}
    </>
  )
}

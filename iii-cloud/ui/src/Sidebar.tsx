import type { Host } from '@iii-dev/console-ui'
import { List, ListGroup, ListGroupLabel, ListItem, PageSidebar, SegmentedControl } from '@iii-dev/console-ui'
import { ExternalLink, ShieldCheck } from 'lucide-react'
import { createContext, type ReactNode, useCallback, useContext, useEffect, useState } from 'react'
import { type Cloud, Feeds, useCloud } from './api'
import { liveOf, needsAction, projectName, projectUrl, roleOf, sha7, type Tone, tilde } from './model'
import { Dot, useLeave } from './parts'
import type { Route } from './route'
import type { DeploymentList, Status } from './types'

const EnvTabs = createContext<ReactNode>(null)
/** The narrow pane's environment tabs (null on a wide one): the page puts them right under its path bar. */
export const useEnvTabs = () => useContext(EnvTabs)

type Env = NonNullable<Status['project']>['environments'][number]
type Feed = Cloud<DeploymentList>

/** Reads one environment's `/deployments` and hands it up: the shell reads each once per refresh. */
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

/** "Live · 9c04e7f", "Not deployed yet · PR #170", "deploying #7": the state line under an environment's name. */
function stateLine(feed: Feed | undefined, s: Status, env: Env): { tone: Tone; text: string } {
  const run = s.deploys.find((d) => d.environment_id === env.id && d.outcome === 'running' && !needsAction(d))
  if (run) return { tone: 'live', text: `deploying #${run.number}` }
  const live = liveOf(feed?.data?.deployments)
  const pr = s.project?.environment?.id === env.id && s.pr?.state === 'OPEN' ? s.pr : null
  const label = live?.label === 'Nothing live yet' ? 'Not deployed yet' : live?.label
  const text = [label, live?.sha ? sha7(live.sha) : null, pr ? `PR #${pr.number}` : null].filter(Boolean).join(' · ')
  return { tone: live?.tone ?? 'ghost', text }
}

/**
 * The project around an environment page: the project's environments with a state line each (a sidebar; on a
 * narrow pane the tabs of the phone board), the folder and account at the bottom, and a link to the project in
 * iii cloud's console, where a new environment is made. It reads every environment's deployments once and
 * shares them (`Feeds`) with the page inside. Opening another environment asks first when something waits on
 * the person here (`useLeave`).
 */
export function ProjectShell({
  host,
  status,
  route,
  go,
  narrow,
  children,
}: {
  host: Host
  status: Status
  route: Extract<Route, { at: 'env' }>
  go: (r: Route) => void
  narrow: boolean
  children: ReactNode
}) {
  const [feeds, setFeeds] = useState<Record<string, Feed>>({})
  const onRead = useCallback((id: string, c: Feed) => setFeeds((f) => ({ ...f, [id]: c })), [])
  const { leave, dialog } = useLeave(host, status)
  const project = status.project
  if (!project) return <>{children}</>
  const deleted = !!status.problems?.some((p) => p.code === 'PROJECT_DELETED')
  const open = (e: Env) => {
    const to = () => go({ at: 'env', env: e.id })
    return e.id === project.environment?.id ? to() : leave(e.name, to)
  }
  const name = projectName(status)
  const team = project.team?.name
  const role = roleOf(status)
  const url = projectUrl(status)
  const envs = project.environments.map((e) => ({ e, line: stateLine(feeds[e.id], status, e) }))
  const readers = deleted
    ? null
    : project.environments.map((e) => (
        <FeedReader key={e.id} host={host} status={status} envId={e.id} onRead={onRead} />
      ))

  // On a narrow pane the environments are tabs; the page inside puts them under its path bar (`useEnvTabs`).
  const tabs =
    narrow && project.environments.length > 1 ? (
      <SegmentedControl
        variant="tabs"
        aria-label={`${name} environments`}
        className="iii-cloud-seg"
        itemClassName="iii-cloud-seg-item"
        value={route.env}
        onChange={(id) => {
          const e = project.environments.find((x) => x.id === id)
          if (e) open(e)
        }}
        options={envs.map(({ e, line }) => ({
          value: e.id,
          label: (
            <span className="iii-cloud-row">
              <Dot tone={line.tone} />
              {e.name}
            </span>
          ),
          icon: false as const,
        }))}
      />
    ) : null

  return (
    <Feeds.Provider value={feeds}>
      <EnvTabs.Provider value={tabs}>
        {readers}
        {narrow ? null : (
          <PageSidebar label={name} narrowMode="inline" narrowBelow={640} defaultWidth={232}>
            <div className="iii-cloud-side">
              <div className="iii-cloud-side-head">
                <span className="iii-cloud-side-name">{name}</span>
                <span className="iii-cloud-psmall">
                  {project.repository}
                  {team ? ` · team ${team}` : ''}
                </span>
              </div>
              <List aria-label={`${name} environments`}>
                <ListGroup>
                  <ListGroupLabel>Environments</ListGroupLabel>
                  {envs.map(({ e, line }) => (
                    <ListItem
                      key={e.id}
                      leading={<Dot tone={line.tone} />}
                      label={e.name}
                      description={line.text || undefined}
                      trailing={e.production ? <ShieldCheck role="img" aria-label="Production" size={16} /> : undefined}
                      selected={route.env === e.id}
                      aria-current={route.env === e.id ? 'page' : undefined}
                      onClick={() => open(e)}
                    />
                  ))}
                </ListGroup>
              </List>
              {url ? (
                <a className="iii-cloud-linkbtn iii-cloud-small" href={url} target="_blank" rel="noreferrer">
                  New environment in iii cloud <ExternalLink size={16} aria-hidden />
                </a>
              ) : null}
              <div className="iii-cloud-side-foot">
                <span className="iii-cloud-psmall" title={project.path}>
                  Folder {tilde(project.path)}
                </span>
                <span className="iii-cloud-psmall">
                  {status.connection.api_key
                    ? `API key ${status.connection.api_key.secret}`
                    : `${status.connection.user?.email ?? 'Signed in'}${role ? ` · ${role}` : ''}`}
                </span>
              </div>
            </div>
          </PageSidebar>
        )}
        {children}
        {dialog}
      </EnvTabs.Provider>
    </Feeds.Provider>
  )
}

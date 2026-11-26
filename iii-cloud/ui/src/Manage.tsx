import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  EmptyState,
  IconButton,
  PageMain,
  Skeleton,
} from '@iii-dev/console-ui'
import { Ellipsis } from 'lucide-react'
import { useCloud } from './api'
import { EnvDeploys } from './EnvDeploys'
import { EnvOverview, liveOf, StateChip, useDeployments } from './EnvOverview'
import { EnvSettings } from './EnvSettings'
import { useActions, useLocked, useStale } from './hub'
import { projectUrl, shortDate, signInRequired } from './model'
import { ErrorPanel, Fresh, Notice, PageAction, PathBar, StaleNotice, useSwitch } from './parts'
import { type AreaProps, envState, pathOf, type RouteAt } from './route'
import { useEnvTabs } from './Sidebar'
import type { Status, WorkerError } from './types'

type ManageProps = AreaProps<RouteAt<'env' | 'secrets'>>

/** An environment of the project in use (its sidebar and tabs are the shell's, CloudPage) or its secrets and variables. */
export function ManageArea(props: ManageProps) {
  const { host, status, route } = props
  const { error, call } = useActions(host)
  const stale = useStale(host)
  const project = status.project
  if (!project) return null
  const deleted = status.problems?.find((p) => p.code === 'PROJECT_DELETED')
  const page = deleted ? (
    <div className="iii-cloud-body">
      <Deleted host={host} status={status} problem={deleted} />
    </div>
  ) : (
    <EnvPage {...props} />
  )
  const notes = [
    route.at === 'env' && route.note ? <Notice key="note" variant="info" headline={route.note} /> : null,
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
    <PageMain className="iii-cloud-main" data-stale={stale ? '' : undefined}>
      {notes.length ? <div className="iii-cloud-body iii-cloud-notes">{notes}</div> : null}
      {page}
    </PageMain>
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
      ? `It can be restored until ${shortDate(rec.restorable_until)} in the iii cloud console. `
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
function ElsewhereEnv({ host, status, route, go }: ManageProps) {
  const { error, switchTo, dialog } = useSwitch(host, status, go)
  // Out of date, or iii cloud wants a fresh sign-in: no opening it (the notes above say why).
  const locked = useLocked(host, status)
  const at = holderOf(status, route.env)
  return (
    <div className="iii-cloud-body">
      {error ? <ErrorPanel error={error} /> : null}
      {at ? (
        <EmptyState
          title={`${at.env} is in ${at.name}`}
          description="Open that project to see it. A running check, a deployment and your agent in this one keep going."
          action={
            locked
              ? undefined
              : { label: `Open ${at.name}`, onClick: () => switchTo(at.key, at.name, at.payload, at.env) }
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

/**
 * One environment: what is live, its deployments (`deploy` opens one's record) and, for now, today's content of
 * the old Overview; or its secrets and variables. The areas of the redesign replace the content, not this frame.
 */
function EnvPage(props: ManageProps) {
  const { host, status, route, go, narrow } = props
  const project = status.project
  const state = envState(status, route.env)
  const env = project?.environments.find((e) => e.id === route.env)
  const locked = useLocked(host, status)
  const feed = useDeployments(host, status, route.env)
  const tabs = useEnvTabs()
  if (state === 'unknown' || !project || !env) return <ElsewhereEnv {...props} />
  const selected = state === 'selected'
  const live = liveOf(feed.data?.deployments)
  const url = selected ? project.environment?.url?.replace(/^https?:\/\//, '') : null
  const gone = selected ? status.problems?.find((p) => p.code === 'ENVIRONMENT_DELETED') : undefined
  const deploy = () => go({ at: 'deploy', env: route.env })
  const consoleUrl = projectUrl(status)
  const tabProps = { ...props, envId: route.env }
  const body = !selected ? (
    <Skeleton className="iii-cloud-skeleton" />
  ) : gone ? (
    <Deleted host={host} status={status} problem={gone} />
  ) : route.at === 'secrets' ? (
    <EnvSettings {...tabProps} />
  ) : route.deploy ? (
    <EnvDeploys {...tabProps} />
  ) : (
    <>
      <EnvOverview {...tabProps} />
      <EnvDeploys {...tabProps} />
    </>
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
        rule={!tabs}
        end={
          <>
            <Fresh host={host} status={status} narrow={narrow} />
            {route.at !== 'env' ? null : narrow ? (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <IconButton label="More actions" variant="ghost">
                    <Ellipsis />
                  </IconButton>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem disabled={locked} onSelect={deploy}>
                    Deploy to {env.name}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            ) : (
              <PageAction disabled={locked} onClick={deploy}>
                Deploy to {env.name}
              </PageAction>
            )}
          </>
        }
      >
        {live ? <StateChip live={live} /> : null}
        {url && !narrow ? <span className="iii-cloud-mono iii-cloud-url">{url}</span> : null}
      </PathBar>
      {tabs}
      <div className="iii-cloud-body iii-cloud-mng iii-cloud-scroll">
        {body}
        {route.at === 'env' && selected && !gone ? (
          <p className="iii-cloud-desc">
            <button type="button" className="iii-cloud-linkbtn" onClick={() => go({ at: 'secrets', env: route.env })}>
              Secrets and variables
            </button>
            {consoleUrl ? (
              <>
                {' · Machines, domains, rollback and a new environment are in '}
                <a href={consoleUrl} target="_blank" rel="noreferrer">
                  iii cloud's console
                </a>
                .
              </>
            ) : null}
          </p>
        ) : null}
      </div>
    </>
  )
}

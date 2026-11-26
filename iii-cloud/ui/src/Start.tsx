// Start (§3 of the navigation plan): the projects on this machine, the account, and the one way to switch
// (a row's Switch) and to add (the two add actions). Every read is a GET through `iii-cloud::api`.

import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  Chip,
  EmptyState,
  List,
  ListItem,
  PageMain,
  Panel,
  PanelBody,
  PanelHeader,
  Skeleton,
} from '@iii-dev/console-ui'
import { ChevronRight, Cloud } from 'lucide-react'
import type { ReactNode } from 'react'
import { useCloud } from './api'
import { ConnectView } from './ConnectView'
import { type DeploymentList, liveOf, StateChip } from './EnvOverview'
import { useActions, useLocked } from './hub'
import { ago, minutesLabel, needsAction, plural, sha7, shortDate, tilde } from './model'
import {
  Dot,
  ErrorPanel,
  Fresh,
  LinkButton,
  Mono,
  moving,
  Notice,
  PathBar,
  Spinner,
  StaleNotice,
  useFollow,
  useNow,
} from './parts'
import { type AreaProps, pathOf, type Route, type RouteAt } from './route'
import type { Status } from './types'

/** One project on this machine: a folder and a cloud project, and how the worker takes it up again. */
export type Place = {
  key: string
  current: boolean
  path: string
  repository: string
  cp: string | null
  name: string
  teamId: string | null
  teamName: string | null
  /** The environment it was on (the selected one for the current project), and its name when known. */
  env: string | null
  envName?: string | null
  mode: 'worktree' | 'checkout'
}

const MAX = 9

/** An older recent entry has only the team's name: it counts when exactly one of the person's teams has it. */
function teamByName(s: Status, name: string | null | undefined): string | null {
  const teams = s.connection.teams.filter((t) => t.name === name)
  return teams.length === 1 ? teams[0].id : null
}

/**
 * The rows, in order: the project in use, the recent ones (newest first), then the other cloud projects of
 * this folder's repository; one per folder and cloud project, at most nine. An API key reaches one row.
 */
export function placesOf(s: Status): Place[] {
  const p = s.project
  const out: Place[] = []
  const add = (x: Omit<Place, 'key'>) => {
    const key = `${x.path}:${x.cp ?? ''}`
    if (out.length < MAX && !out.some((o) => o.key === key)) out.push({ ...x, key })
  }
  const mode = p?.workspace?.mode ?? 'worktree'
  if (p) {
    add({
      current: true,
      path: p.path,
      repository: p.repository,
      cp: p.cloud_project?.id ?? null,
      name: p.cloud_project?.name ?? p.repository,
      teamId: p.team?.id ?? null,
      teamName: p.team?.name ?? null,
      env: p.environment?.id ?? null,
      mode,
    })
  }
  if (s.connection.api_key) return out
  const used = (iso?: string) => Date.parse(iso ?? '') || 0
  for (const r of [...(s.recent_projects ?? [])].sort((a, b) => used(b.used_at) - used(a.used_at))) {
    add({
      current: false,
      path: r.path,
      repository: r.repository,
      cp: r.cloud_project_id,
      name: r.cloud_project_name || r.repository,
      teamId: r.team_id ?? teamByName(s, r.team_name),
      teamName: r.team_name ?? null,
      env: r.environment_id ?? null,
      envName: r.environment_name ?? null,
      mode: r.mode ?? 'worktree',
    })
  }
  for (const c of p?.cloud_projects ?? []) {
    if (c.deleted || !p) continue
    add({
      current: false,
      path: p.path,
      repository: p.repository,
      cp: c.id,
      name: c.name,
      teamId: c.team?.id ?? p.team?.id ?? null,
      teamName: c.team?.name ?? p.team?.name ?? null,
      env: null,
      mode,
    })
  }
  return out
}

/** Something was live once: a ready deploy here, or the cloud project says it deployed. */
function everLive(s: Status): boolean {
  const p = s.project
  const cp = p?.cloud_projects?.find((c) => c.id === p.cloud_project?.id)
  return s.deploys.some((d) => d.outcome === 'ready') || !!cp?.last_deployment_at
}

type CloudEnv = { id: string; name: string; production: boolean; status?: string }
type Deletions = {
  deletions: { kind: string; project?: { id: string } | null; restorable: boolean; restorable_until: string | null }[]
}

/** "dev · serves 9c04e7f · deploy #6 · 2h ago · 3 environments", from the env's first page of deployments. */
function liveLine(env: string, list: DeploymentList['deployments'], count: number | null): string {
  const newest = list[0]
  const current = list.find((d) => d.current)
  const fresh = newest && newest !== current ? newest : null
  const what =
    fresh && (fresh.status === 'queued' || fresh.status === 'in_progress')
      ? `deploying #${fresh.number}`
      : fresh?.status === 'needs_action'
        ? `deploy #${fresh.number} needs your action`
        : fresh?.status === 'failed'
          ? `deploy #${fresh.number} failed ${ago(fresh.finished_at ?? fresh.created_at)}`
          : current
            ? `serves ${sha7(current.commit.sha)} · deploy #${current.number} · ${ago(current.finished_at ?? current.created_at)}`
            : 'nothing live yet'
  return [env, what, count === null ? null : plural(count, 'environment')].filter(Boolean).join(' · ')
}

type RowProps = {
  host: Host
  status: Status
  place: Place
  narrow: boolean
  /** Not signed in: no reads, no live state, and no Switch. */
  offline: boolean
  locked: boolean
  switching: string | null
  go: (r: Route) => void
  switchTo: ReturnType<typeof useFollow>['switchTo']
  /** A deploy going on: its follow page (another project's switches first). */
  follow: ReturnType<typeof useFollow>['follow']
}

function PlaceRow({ host, status, place, narrow, offline, locked, switching, go, switchTo, follow }: RowProps) {
  const key = status.connection.api_key
  const p = status.project
  const team = key ? key.team_id : place.teamId
  // The project in use: its environments are in the status. Another one: iii cloud lists them.
  const envs = useCloud<{ environments: CloudEnv[] }>(
    host,
    status,
    null,
    !offline && !place.current && team && place.cp ? `/teams/${team}/projects/${place.cp}/environments` : null,
  )
  const listed = place.current ? p?.environments : envs.data?.environments.filter((e) => e.status !== 'deleting')
  const env = place.current
    ? (p?.environment ?? null)
    : (listed?.find((e) => e.id === place.env) ?? listed?.find((e) => e.production) ?? listed?.[0] ?? null)
  const deploys = useCloud<DeploymentList>(
    host,
    status,
    null,
    !offline && team && env ? `/teams/${team}/environments/${env.id}/deployments` : null,
  )
  const gone =
    (!place.current && envs.error?.status === 404) ||
    (place.current && !!status.problems?.some((x) => x.code === 'PROJECT_DELETED'))
  const deletions = useCloud<Deletions>(host, status, null, gone && team ? `/teams/${team}/deletions` : null)
  const list = deploys.data?.deployments
  const live = liveOf(list)
  const setUp = place.current && !!p?.workspace && (everLive(status) || !!list?.some((d) => d.current))
  // The project in use, before its deployments say whether it went live: neither Open nor Continue setup yet.
  const pending = place.current && !setUp && !!p?.workspace && deploys.loading && !list
  const readError = envs.error && !gone ? envs.error : deploys.error
  // A deploy going on from this folder (any of its environments): the row says so and follows it.
  const now = useNow(15_000)
  const going = moving(status).find(
    (m) =>
      !needsAction(m.deploy) &&
      (place.current ? !m.other : m.path === place.path && (!place.cp || m.cloudProject === place.cp)),
  )
  const run = going?.deploy
  const toRun = () => going && follow(going, place.key)

  let line1: ReactNode
  if (gone) {
    const rec = deletions.data?.deletions.find((d) => d.kind === 'project' && d.project?.id === place.cp)
    line1 =
      rec?.restorable && rec.restorable_until
        ? `restorable until ${shortDate(rec.restorable_until)}`
        : 'deleted in iii cloud'
  } else if (going && run) {
    line1 = `deploying #${run.number}${going.env ? ` to ${going.env}` : ''} · ${minutesLabel(now - Date.parse(run.created_at))}`
  } else if (offline) line1 = "Sign in to see what's live"
  else if (!team) line1 = "Switch to see what's live"
  else if (readError) line1 = `Can't read iii cloud: ${readError.message}`
  else if (env && list) line1 = liveLine(env.name, list, key ? null : (listed?.length ?? null))
  else if (listed && !env) line1 = place.current ? 'no environment chosen yet' : 'no environment yet'
  else line1 = <Skeleton className="iii-cloud-skeleton-row" />

  // The folder, how the worker works in it (not on a phone), the team: the repository is in the name or the folder.
  const teamWord = place.teamName ? ` · team ${place.teamName}` : ''
  const line2 = narrow ? (
    `${tilde(place.path)}${teamWord}`
  ) : (
    <>
      <Mono>{tilde(place.path)}</Mono>
      {` · ${place.mode === 'checkout' ? 'edits this folder' : 'separate copy'}${teamWord}`}
    </>
  )
  const name = key ? place.repository : place.name
  // The project in use says so beside its name; what it serves is its first line.
  const chip = gone ? (
    <Chip>Deleted</Chip>
  ) : place.current ? (
    <Chip>
      <Dot tone="ghost" />
      In use
    </Chip>
  ) : live && !offline ? (
    <StateChip live={live} />
  ) : null
  const mine = switching === place.key
  const listedEnv = listed?.some((e) => e.id === place.env) ? place.env : null
  const doSwitch = () =>
    switchTo(
      place.key,
      name,
      {
        path: place.path,
        ...(place.cp ? { cloud_project_id: place.cp } : {}),
        ...(listedEnv ? { environment_id: listedEnv } : {}),
        mode: place.mode,
      },
      place.envName,
    )
  const open = () => go(setUp ? { at: 'env', env: '' } : { at: 'setup' })
  const cloud = `${status.connection.cloud_url.replace(/\/$/, '')}/${status.connection.teams.find((t) => t.id === team)?.slug ?? ''}`
  // Until iii cloud lists the row's environments, Switch can't tell which one to ask for.
  const reading = !place.current && envs.loading && !envs.data && !envs.error
  const off = !!switching || offline || locked || reading

  const label = (
    <span className="iii-cloud-row iii-cloud-start-name">
      {name}
      {chip}
    </span>
  )
  const description = (
    <span className="iii-cloud-start-lines">
      <span>{line1}</span>
      <span className="iii-cloud-faint">{line2}</span>
    </span>
  )
  const leading = <Dot tone={gone || offline ? 'ghost' : run ? 'live' : (live?.tone ?? 'ghost')} />

  if (narrow && !gone) {
    // On a phone the row is the action: its word and a chevron say which.
    const word = run ? 'Follow' : place.current ? (setUp ? 'Open' : 'Continue') : mine ? 'Switching…' : 'Switch'
    return (
      <ListItem
        className="iii-cloud-start-row"
        aria-label={
          run
            ? `Follow deploy #${run.number} of ${name}`
            : place.current
              ? `${word} ${name}`
              : `Switch the worker to ${name} in ${tilde(place.path)}`
        }
        aria-busy={reading || mine || undefined}
        disabled={run ? !!switching || locked : place.current ? pending || (setUp && offline) : off}
        label={label}
        description={description}
        trailing={
          <span className="iii-cloud-start-word">
            {word}
            <ChevronRight aria-hidden />
          </span>
        }
        onClick={run ? toRun : place.current ? open : doSwitch}
      />
    )
  }
  const trailing = gone ? (
    <LinkButton href={cloud}>Open in iii cloud</LinkButton>
  ) : run ? (
    // Follow the deploy first; the row's usual action (Open, Continue setup, Switch) stays, quiet, beside it.
    <span className="iii-cloud-row">
      {place.current ? (
        <Button size="sm" variant="ghost" disabled={!!switching} onClick={open}>
          {setUp ? 'Open' : 'Continue setup'}
        </Button>
      ) : (
        <Button
          size="sm"
          variant="ghost"
          aria-label={`Switch the worker to ${name} in ${tilde(place.path)}`}
          disabled={off}
          onClick={doSwitch}
        >
          Switch
        </Button>
      )}
      <Button
        size="sm"
        variant="primary"
        aria-busy={mine || undefined}
        disabled={!!switching || (!place.current && locked)}
        onClick={toRun}
      >
        {mine ? <Spinner /> : null}
        Follow deploy
      </Button>
    </span>
  ) : place.current ? (
    pending ? null : setUp ? (
      <Button size="sm" variant="secondary" disabled={!!switching || offline} onClick={open}>
        Open
      </Button>
    ) : (
      <Button size="sm" variant="primary" disabled={!!switching} onClick={open}>
        Continue setup
      </Button>
    )
  ) : (
    <Button
      size="sm"
      variant="secondary"
      aria-label={`Switch the worker to ${name} in ${tilde(place.path)}`}
      aria-busy={reading || mine || undefined}
      disabled={off}
      onClick={doSwitch}
    >
      {mine ? (
        <>
          <Spinner />
          Switching…
        </>
      ) : reading ? (
        <>
          <Spinner />
          Switch
        </>
      ) : (
        'Switch'
      )}
    </Button>
  )
  return (
    <ListItem
      as="div"
      className="iii-cloud-start-row"
      leading={narrow ? undefined : leading}
      label={label}
      description={description}
      trailing={trailing}
    />
  )
}

/** The account in one row: who is signed in and on which teams; else the way to sign in. */
function Account({ host, status, narrow }: { host: Host; status: Status; narrow: boolean }) {
  const { busy, error, call } = useActions(host)
  const c = status.connection
  const signIn = (label: string, variant: 'primary' | 'secondary') => (
    <Button
      size="sm"
      variant={variant}
      disabled={busy === 'connect'}
      onClick={() => void call('connect', 'iii-cloud::connect', { method: 'browser' })}
    >
      {label}
    </Button>
  )
  if (c.status === 'waiting') return <ConnectView host={host} connection={c} />
  if (c.status !== 'connected') {
    const expired = c.status === 'expired'
    return (
      <>
        <Notice
          variant="warn"
          headline={expired ? 'Your sign-in to iii cloud expired' : 'Not signed in to iii cloud'}
          detail="Sign in to see what each project serves and to switch between them."
          action={expired ? signIn('Sign in again', 'primary') : signIn('Sign in with your browser', 'primary')}
        />
        {error ? <ErrorPanel error={error} /> : null}
      </>
    )
  }
  const who = c.user?.email ?? c.user?.name ?? 'you'
  const teamsWord = c.teams.length === 1 ? 'team' : 'teams'
  const role = (r: string) => r.charAt(0).toUpperCase() + r.slice(1)
  if (narrow) {
    // A phone: one line, the team names only.
    return (
      <>
        <div className="iii-cloud-start-account">
          <span className="iii-cloud-grow">
            {c.api_key
              ? `API key ${c.api_key.secret} · reaches one environment`
              : `Signed in as ${who}${c.teams.length ? ` · ${teamsWord} ${c.teams.map((t) => t.name).join(', ')}` : ''}`}
          </span>
          {c.api_key ? signIn('Sign in with your browser', 'secondary') : null}
        </div>
        {error ? <ErrorPanel error={error} /> : null}
      </>
    )
  }
  return (
    <Panel aria-label="Account">
      <PanelHeader className="iii-cloud-panel-head">Account</PanelHeader>
      <PanelBody className="iii-cloud-panel-body">
        <div className="iii-cloud-start-kv">
          <span className="iii-cloud-start-key">{c.api_key ? 'API key' : 'Signed in'}</span>
          <span className="iii-cloud-start-val">
            {c.api_key
              ? `${c.api_key.secret} · reaches one environment`
              : `${who}${c.teams.length ? ` · ${teamsWord} ${c.teams.map((t) => `${t.name} (${role(t.role)})`).join(', ')}` : ''}`}
          </span>
          {c.api_key ? signIn('Sign in with your browser', 'secondary') : null}
        </div>
        {error ? <ErrorPanel error={error} /> : null}
      </PanelBody>
    </Panel>
  )
}

const FOOTNOTE =
  'Switching moves this worker to that folder and environment: checks, deploys and this page follow it. A running check, a deploy and your agent keep going; switch back to see them.'
const FOOTNOTE_PHONE =
  'Switching moves this worker to that folder and environment. A running check, a deploy and your agent keep going.'

/** The projects on this machine (Start): desktop one column, phone one row per project. */
export function StartArea({ host, status, route, go, narrow }: AreaProps<RouteAt<'start'>>) {
  const locked = useLocked(host, status)
  const { switching, error, switchTo, follow, dialog } = useFollow(host, status, go)
  const places = placesOf(status)
  const key = status.connection.api_key
  const offline = status.connection.status !== 'connected'
  // Adding keeps the project in use until Continue; with none in use, it is the first setup.
  const using = !!status.project?.workspace
  const addFolder: Route = using ? { at: 'setup', step: 'confirm', adding: true } : { at: 'setup', step: 'confirm' }
  const addCloud: Route = using
    ? { at: 'setup', step: 'confirm', adding: true, newProject: true }
    : { at: 'setup', step: 'confirm', newProject: true }
  const rows = places.map((p) => (
    <PlaceRow
      key={p.key}
      host={host}
      status={status}
      place={p}
      narrow={narrow}
      offline={offline}
      locked={locked}
      switching={switching}
      go={go}
      switchTo={switchTo}
      follow={follow}
    />
  ))
  const list = places.length ? (
    <List aria-label="Projects on this machine" className={narrow ? 'iii-cloud-phonelist' : undefined}>
      {rows}
    </List>
  ) : (
    <EmptyState
      icon={Cloud}
      title="No projects on this machine yet"
      description="Add the folder of a project: the worker reads it and checks it before the first deploy."
      action={key ? undefined : { label: 'Add a project', onClick: () => go(addFolder) }}
    />
  )
  return (
    <PageMain className="iii-cloud-main">
      <PathBar
        path={pathOf(route, status)}
        host={host}
        status={status}
        route={route}
        go={go}
        narrow={narrow}
        rule
        end={<Fresh host={host} status={status} />}
      >
        {narrow ? null : <span className="iii-cloud-desc">on this machine</span>}
      </PathBar>
      <div className="iii-cloud-body iii-cloud-scroll iii-cloud-start">
        <StaleNotice host={host} status={status} />
        <Account host={host} status={status} narrow={narrow} />
        {error ? <ErrorPanel error={error} /> : null}
        {narrow ? (
          <>
            {places.length ? (
              <Panel>
                <PanelHeader className="iii-cloud-panel-head">On this machine</PanelHeader>
                {list}
              </Panel>
            ) : (
              list
            )}
            {key || !places.length ? null : (
              <div className="iii-cloud-start-adds">
                <Button variant="secondary" size="lg" onClick={() => go(addFolder)}>
                  Add a project
                </Button>
                <Button variant="ghost" size="lg" className="iii-cloud-start-ink" onClick={() => go(addCloud)}>
                  New project in iii cloud
                </Button>
              </div>
            )}
          </>
        ) : (
          <Panel aria-label="Projects on this machine">
            <PanelHeader className="iii-cloud-panel-head">
              <span>Projects on this machine</span>
              {key ? null : (
                <span className="iii-cloud-panel-end">
                  <Button
                    size="sm"
                    variant="secondary"
                    className="iii-cloud-start-headbtn"
                    onClick={() => go(addCloud)}
                  >
                    New project in iii cloud…
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    className="iii-cloud-start-headbtn"
                    onClick={() => go(addFolder)}
                  >
                    Add a project…
                  </Button>
                </span>
              )}
            </PanelHeader>
            {places.length ? list : <PanelBody>{list}</PanelBody>}
          </Panel>
        )}
        {places.length > 1 ? <p className="iii-cloud-desc">{narrow ? FOOTNOTE_PHONE : FOOTNOTE}</p> : null}
      </div>
      {dialog}
    </PageMain>
  )
}

// Projects: the projects on this machine, one "Open" for each, and the one way to add. Every read is a GET
// through `iii-cloud::api`. The pill is the state of the project's environment.

import type { Host } from '@iii-dev/console-ui'
import { Button, EmptyState, List, ListItem, PageMain, Skeleton } from '@iii-dev/console-ui'
import { ChevronRight, Cloud } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { useCloud } from './api'
import { ConnectView } from './ConnectView'
import { useActions, useLocked } from './hub'
import {
  ago,
  liveOf,
  minutesLabel,
  needsAction,
  plainDetail,
  plainMessage,
  plural,
  sha7,
  shortDate,
  type Tone,
  tilde,
  whatWaits,
} from './model'
import {
  ErrorPanel,
  Fresh,
  LinkButton,
  Mono,
  moving,
  Notice,
  PathBar,
  SectionCard,
  Spinner,
  StaleNotice,
  StatePill,
  useNow,
  useSwitch,
} from './parts'
import { type AreaProps, pathOf, type Route, type RouteAt } from './route'
import type { DeploymentList, Status } from './types'

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

type CloudEnv = { id: string; name: string; production: boolean; status?: string }
type Deletions = {
  deletions: { kind: string; project?: { id: string } | null; restorable: boolean; restorable_until: string | null }[]
}

/** What the environment serves or is doing, from its first page of deployments: "dev · serves 9c04e7f · deployment #6 · 2h ago". */
function liveLine(env: string, list: DeploymentList['deployments']): string {
  const newest = list[0]
  const current = list.find((d) => d.current)
  const fresh = newest && newest !== current ? newest : null
  const what =
    fresh && (fresh.status === 'queued' || fresh.status === 'in_progress')
      ? `deployment #${fresh.number} is running`
      : fresh?.status === 'needs_action'
        ? `deployment #${fresh.number} needs your action`
        : fresh?.status === 'failed'
          ? `deployment #${fresh.number} failed ${ago(fresh.finished_at ?? fresh.created_at)}${current ? ` · still serves #${current.number}` : ''}`
          : current
            ? `serves ${sha7(current.commit.sha)} · deployment #${current.number} · ${ago(current.finished_at ?? current.created_at)}`
            : 'not deployed yet'
  return [env, what].join(' · ')
}

/** The environment's state as the pill says it: the shared words, with "Needs you" and "Not deployed yet". */
function pillOf(live: ReturnType<typeof liveOf>): { label: string; tone: Tone } | null {
  if (!live) return null
  if (live.label === 'Nothing live yet') return { label: 'Not deployed yet', tone: 'ghost' }
  if (live.label === 'Last deployment failed') return { label: 'Failed', tone: 'alert' }
  if (live.label === 'Needs your action') return { label: 'Needs you', tone: 'warn' }
  return { label: live.label, tone: live.tone }
}

type RowProps = {
  host: Host
  status: Status
  place: Place
  narrow: boolean
  /** Not signed in: no reads, no live state, and nothing to open that is not in use. */
  offline: boolean
  locked: boolean
  switching: string | null
  go: (r: Route) => void
  open: (key: string, name: string, payload: Record<string, unknown>, envName?: string | null) => void
}

function PlaceRow({ host, status, place, narrow, offline, locked, switching, go, open }: RowProps) {
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
  const readError = envs.error && !gone ? envs.error : deploys.error
  // A deploy going on from this folder (any of its environments): the row says so.
  const now = useNow(15_000)
  const going = moving(status).find(
    (m) =>
      !needsAction(m.deploy) &&
      (place.current ? !m.other : m.path === place.path && (!place.cp || m.cloudProject === place.cp)),
  )
  const run = going?.deploy
  const waits = place.current ? whatWaits(status) : []

  let line1: ReactNode
  if (gone) {
    const rec = deletions.data?.deletions.find((d) => d.kind === 'project' && d.project?.id === place.cp)
    line1 =
      rec?.restorable && rec.restorable_until
        ? `restorable until ${shortDate(rec.restorable_until)}`
        : 'deleted in iii cloud'
  } else if (going && run) {
    line1 = `${going.env ?? env?.name ?? 'deploying'} · deployment #${run.number} is running · ${minutesLabel(now - Date.parse(run.created_at))} so far`
  } else if (offline) line1 = "Sign in to see what's live"
  // Only the folder and the names are known: say what exists, and how to see the rest.
  else if (!team) line1 = place.envName ? `${place.envName} · open it to see what's live` : "Open it to see what's live"
  else if (readError) line1 = `Can't read iii cloud: ${readError.message}`
  else if (env && list) {
    line1 = liveLine(env.name, list)
    if (waits.length) line1 = `${line1} · ${waits.join(', ')}`
  } else if (listed && !env) line1 = 'no environment yet'
  else line1 = <Skeleton className="iii-cloud-skeleton-row" />

  const teamWord = place.teamName ? ` · team ${place.teamName}` : ''
  const line2 = narrow ? (
    `${tilde(place.path)}${teamWord}`
  ) : (
    <>
      <Mono>{tilde(place.path)}</Mono>
      {teamWord}
    </>
  )
  const name = key ? place.repository : place.name
  const state = gone
    ? { label: 'Deleted', tone: 'ghost' as const }
    : run
      ? { label: 'Deploying', tone: 'live' as const }
      : waits.length
        ? { label: 'Needs you', tone: 'warn' as const }
        : offline
          ? null
          : pillOf(live)
  const pill = state ? <StatePill state={state} /> : null
  const mine = switching === place.key
  const listedEnv = listed?.some((e) => e.id === place.env) ? place.env : null
  const doOpen = () =>
    place.current
      ? go(p?.workspace ? { at: 'env', env: '' } : { at: 'setup' })
      : open(
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
  const cloud = `${status.connection.cloud_url.replace(/\/$/, '')}/${status.connection.teams.find((t) => t.id === team)?.slug ?? ''}`
  // Until iii cloud lists the row's environments, Open can't tell which one to ask for.
  const reading = !place.current && envs.loading && !envs.data && !envs.error
  const off = !!switching || (!place.current && (offline || locked || reading))

  const label = (
    <span className="iii-cloud-row iii-cloud-start-name">
      {name}
      {narrow ? null : pill}
    </span>
  )
  const description = (
    <span className="iii-cloud-start-lines">
      <span>{line1}</span>
      <span className="iii-cloud-faint">{line2}</span>
    </span>
  )

  if (narrow && !gone) {
    // On a phone the whole row opens the project; the pill says its state.
    return (
      <ListItem
        className="iii-cloud-start-row"
        aria-label={`Open ${name} in ${tilde(place.path)}`}
        aria-busy={reading || mine || undefined}
        disabled={off}
        label={label}
        description={description}
        trailing={
          <span className="iii-cloud-start-word">
            {pill}
            <ChevronRight aria-hidden />
          </span>
        }
        onClick={doOpen}
      />
    )
  }
  return (
    <ListItem
      as="div"
      className="iii-cloud-start-row"
      label={label}
      description={description}
      trailing={
        gone ? (
          <LinkButton href={cloud}>Open in iii cloud</LinkButton>
        ) : (
          <Button
            size="sm"
            variant="secondary"
            aria-label={`Open ${name} in ${tilde(place.path)}`}
            aria-busy={reading || mine || undefined}
            disabled={off}
            onClick={doOpen}
          >
            {mine || reading ? <Spinner /> : null}
            Open
          </Button>
        )
      }
    />
  )
}

/** Who is signed in, and on which teams: the path bar's words. */
function whoIs(s: Status): string {
  const c = s.connection
  const who = c.user?.email ?? c.user?.name ?? 'you'
  return `${who}${c.teams.length ? ` · ${c.teams.length === 1 ? 'team' : 'teams'} ${c.teams.map((t) => t.name).join(', ')}` : ''}`
}

/** What is not in order with the account: a sign-in under way, signed out, expired, or an API key (one environment only). */
function Account({ host, status }: { host: Host; status: Status }) {
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
          detail="Sign in to see what each project serves and to open them."
          action={expired ? signIn('Sign in again', 'primary') : signIn('Sign in with your browser', 'primary')}
        />
        {error ? <ErrorPanel error={error} /> : null}
      </>
    )
  }
  if (!c.api_key) return error ? <ErrorPanel error={error} /> : null
  return (
    <>
      <div className="iii-cloud-start-account">
        <span className="iii-cloud-grow">{`API key ${c.api_key.secret} · reaches one environment`}</span>
        {signIn('Sign in with your browser', 'secondary')}
      </div>
      {error ? <ErrorPanel error={error} /> : null}
    </>
  )
}

const FOOTNOTE =
  "Opening a project doesn't stop the others. A check, a deploy and your agent keep going, and the project's row says what they are doing."
const FOOTNOTE_PHONE =
  'The whole row opens the project. A check, a deploy and your agent keep going on this machine, and the row says what they are doing.'

/** The projects on this machine: desktop one column, phone one row per project. */
export function StartArea({ host, status, route, go, narrow }: AreaProps<RouteAt<'start'>>) {
  const locked = useLocked(host, status)
  const { switching, error, switchTo, dialog } = useSwitch(host, status, go)
  const [tried, setTried] = useState('')
  const places = placesOf(status)
  const key = status.connection.api_key
  const offline = status.connection.status !== 'connected'
  // Adding keeps the project in use until the button of the confirmation; with none in use, it is the first time.
  const addRoute: Route = status.project?.workspace
    ? { at: 'setup', step: 'confirm', adding: true }
    : { at: 'setup', step: 'confirm' }
  const open: RowProps['open'] = (k, name, payload, envName) => {
    setTried(name)
    switchTo(k, name, payload, envName)
  }
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
      open={open}
    />
  ))
  const list = (
    <List aria-label="Projects on this machine" className={narrow ? 'iii-cloud-phonelist' : undefined}>
      {rows}
    </List>
  )
  const add = (size: 'sm' | 'lg', variant: 'primary' | 'secondary') => (
    <Button size={size} variant={variant} onClick={() => go(addRoute)}>
      Add a project
    </Button>
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
        end={
          <>
            {narrow || offline ? null : <span className="iii-cloud-desc">{whoIs(status)}</span>}
            <Fresh host={host} status={status} narrow={narrow} />
          </>
        }
      >
        {narrow ? null : <span className="iii-cloud-desc">on this machine</span>}
      </PathBar>
      <div className="iii-cloud-body iii-cloud-scroll iii-cloud-start">
        <StaleNotice host={host} status={status} />
        <Account host={host} status={status} />
        {error ? (
          <Notice
            variant="alert"
            headline={`Couldn't open ${tried || 'the project'}`}
            detail={[plainMessage(error), plainDetail(error)].filter(Boolean).join(' ')}
          />
        ) : null}
        {narrow && !offline && !key ? <p className="iii-cloud-desc">{`Signed in as ${whoIs(status)}`}</p> : null}
        {places.length ? (
          narrow ? (
            <>
              {list}
              {key ? null : <div className="iii-cloud-start-adds">{add('lg', 'secondary')}</div>}
            </>
          ) : (
            <SectionCard title={plural(places.length, 'project')} end={key ? undefined : add('sm', 'primary')} flush>
              {list}
            </SectionCard>
          )
        ) : (
          <EmptyState
            icon={Cloud}
            title="No projects on this machine yet"
            description="Add the folder of a project. The page reads it and checks it before the first deploy."
            action={key ? undefined : { label: 'Add a project', onClick: () => go(addRoute) }}
          />
        )}
        {places.length > 1 ? <p className="iii-cloud-desc">{narrow ? FOOTNOTE_PHONE : FOOTNOTE}</p> : null}
      </div>
      {dialog}
    </PageMain>
  )
}

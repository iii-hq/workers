import { formatDuration, formatRelative, unwrapEnvelope } from '@iii-dev/console-ui/format'
import { clock } from './asked'
import type {
  Check,
  CloudDeployment,
  Connection,
  Deploy,
  DeploymentList,
  HandoffKind,
  Members,
  Problem,
  Run,
  Stage,
  Status,
  WorkerError,
} from './types'

export const PAGE_ID = 'iii-cloud'
export const EVENTS_FN = 'iii::iii-cloud-ui::events'
export const CALL_IDS = ['connect', 'check', 'secret', 'deploy', 'watch'].map((n) => `iii-cloud::${n}`)

export const sha7 = (sha: string) => sha.slice(0, 7)

/**
 * What a deploy takes from, in running text: the branch the person works on, else the folder's, else the commit.
 * The worker's own `iii-cloud/ready-*` branch is never named: it reads "<env>'s branch".
 */
export function sourceLabel(s: Status, env: string): string {
  const p = s.project
  const own = (b: string | null | undefined) => (b && !b.startsWith('iii-cloud/ready-') ? b : null)
  const label = own(p?.workspace?.source) ?? own(p?.checkout.branch)
  return (
    label ??
    (p?.workspace?.source || p?.checkout.branch ? `${env}'s branch` : p ? sha7(p.checkout.head) : 'your folder')
  )
}
export const ago = (iso: string | null | undefined) => {
  const r = iso ? formatRelative(iso) : ''
  return r === 'just now' ? r : r ? `${r} ago` : ''
}
export const duration = (ms: number | null | undefined) => (ms == null ? '–' : formatDuration(ms))

/** `cloud.iii.dev/device` out of the verification URI (never carries the code). */
export function deviceLabel(uri: string): string {
  try {
    const u = new URL(uri)
    return `${u.host}${u.pathname.replace(/\/$/, '')}`
  } catch {
    return uri
  }
}

export function mmss(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds))
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi
/** `<team_id>/<environment_id>`, or the two UUIDs separated by anything. */
export function parseEnvironmentIds(text: string): { team_id: string; environment_id: string } | null {
  const ids = text.match(UUID)
  return ids && ids.length === 2 ? { team_id: ids[0], environment_id: ids[1] } : null
}

export const isConnected = (c: Connection) => c.status === 'connected'

export const needsConnect = (s: Status) => !isConnected(s.connection)
export const needsProject = (s: Status) => !s.project?.workspace

export function envName(s: Status): string {
  const p = s.project
  if (!p?.environment) return ''
  return p.cloud_project ? `${p.cloud_project.name} · ${p.environment.name}` : p.environment.name
}

export function envSlug(s: Status | null, environmentId?: string): string {
  const p = s?.project
  if (p?.cloud_project && p.environment && (!environmentId || p.environment.id === environmentId)) {
    return `${p.cloud_project.name}/${p.environment.name}`
  }
  return environmentId ?? ''
}

export function planLine(c: Check): string {
  const p = c.plan
  if (p.status === 'unavailable') return "iii cloud's plan: unavailable"
  if (p.status === 'not_pushed') return "iii cloud's plan: commit not pushed"
  if (p.status === 'commit_not_planned') return "iii cloud's plan: waits for the merge"
  return p.blocking
    ? `iii cloud's plan: ${p.blocking} to fix${p.at_least ? ', at least' : ''}`
    : "iii cloud's plan is clean"
}

export const peopleToDecide = (c: Check) => c.problems.filter((p: Problem) => p.owner === 'person').length

export function runLine(r: Run): string {
  if (r.status === 'failed') return `Local run failed${r.error ? ` at ${stageLabel(r.error.stage)}` : ''}`
  const hc = r.healthcheck?.status
  return `Local run: ${r.workers.ready} of ${r.workers.total} workers, ${r.routes.count} routes${hc != null ? `, healthcheck ${hc}` : ''}`
}

const RUN_LABELS: Record<Stage['name'], string> = {
  install: 'Frozen install',
  build: 'Build command',
  release: 'Release command',
  compose_up: 'Compose up',
  workers: 'Workers registered their functions',
  routes: 'Routes',
  healthcheck: 'Healthcheck',
}
export const stageLabel = (name: Stage['name']) => RUN_LABELS[name] ?? name

/** The right-hand cell of a run stage: counts for workers and routes, the status for the healthcheck, else the time. */
export function stageTime(r: Run, s: Stage): string {
  if (s.status === 'pending' || s.status === 'skipped') return s.status === 'skipped' ? 'skipped' : ''
  if (s.status === 'running') return 'live'
  if (s.name === 'workers') return `${r.workers.ready} of ${r.workers.total}`
  if (s.name === 'routes') return String(r.routes.count)
  if (s.name === 'healthcheck' && r.healthcheck?.status != null) return String(r.healthcheck.status)
  return duration(s.duration_ms)
}

/** The local run's rows as the canvas words them: what the stage proves, not the command it runs. */
const RUN_ROWS: Record<Stage['name'], string> = {
  install: 'Install exactly what the lock file says',
  build: 'Build the workers',
  release: 'Database, with the rights iii cloud gives your app',
  compose_up: 'Start the workers',
  workers: 'Workers register their functions',
  routes: 'Every route answers',
  healthcheck: 'Health answers',
}

/** A row of the local run: the canvas's words; the command only under a stage that failed, where it says what broke. */
export function stageText(r: Run, s: Stage, postgres = false): { label: string; mono: string | null; note?: string } {
  const mono =
    s.name === 'routes'
      ? r.routes.prefix
      : s.name === 'healthcheck'
        ? r.healthcheck?.path
          ? `GET ${r.healthcheck.path}`
          : null
        : s.name === 'compose_up'
          ? null
          : s.detail
  return {
    label: RUN_ROWS[s.name] ?? stageLabel(s.name),
    mono: s.status === 'failed' ? mono : null,
    note: s.name === 'release' && postgres ? 'Local Postgres with pgvector, for this run only' : undefined,
  }
}

const CLOUD_LABELS: Record<string, string> = {
  validation: 'Validation',
  build: 'Build',
  resources: 'Resources',
  config_secrets: 'Config and secrets',
  workers: 'Workers',
  routes_tls: 'Routes and healthcheck',
}
export function cloudStageLabel(name: string): string {
  return CLOUD_LABELS[name] ?? name
}

const PLAIN: Record<string, string> = {
  WORKTREE_FAILED: 'A separate copy of your code could not be prepared. Nothing was changed in your own files.',
  RUN_BUSY: 'A run is already in progress.',
  IDEMPOTENCY_KEY_REUSED: 'That request was already sent with different content. Try again.',
  WORKER_RESTARTED: 'A restart stopped this while it was running. Run it again.',
  DEVICE_CODE_EXPIRED: 'The sign-in code expired before it was approved. Start the sign-in again.',
  ACCOUNT_BLOCKED: 'This iii cloud account is blocked. Contact your team admin.',
  BROWSER_SIGN_IN_UNAVAILABLE: "Browser sign-in isn't available on this iii cloud yet",
  NOT_SAVED:
    "Not saved: this API key can't write variables and the .env is tracked by git. Set it in the iii cloud console.",
  RUN_PANIC: 'The run stopped unexpectedly. Run it again.',
  PR_HEAD_CHANGED: 'The pull request changed after the check. Check again.',
  PR_NOT_MERGEABLE: "The pull request can't be merged yet. Open it on GitHub to see why, then check again.",
  MAIN_MOVED: 'The base branch moved after the check. Check again before you deploy.',
  PLAN_CHANGED: 'The plan changed after the check. Check again before you deploy.',
  SOURCE_CHANGED: 'The source or the compose file changed after the check. Check again before you deploy.',
  NOT_READY: 'The check no longer holds for this deploy. Check again before you deploy.',
  MERGE_CONFLICT: "Your commits conflict with the environment's branch, so they can't come in without resolving them.",
  MERGE_IN_PROGRESS:
    "A merge is still open in your agent's copy of the code: nothing new comes in until it is finished.",
  WORKTREE_PATH_TAKEN: 'The folder for the separate copy of your code is already used by something else.',
  NO_ACCESS: "You don't have access to that in iii cloud.",
  PUSH_REFUSED: 'GitHub refused the push.',
  PUSH_UNKNOWN: "Couldn't tell whether the push reached GitHub.",
  OWN_BRANCH_TAKEN: "GitHub already has a branch of the environment's own name with other commits. Nothing changed.",
  BRANCH_NOT_MOVED: 'The environment could not move to its own branch. Nothing was deployed. Deploy again to retry.',
  CLOUD_ERROR: 'iii cloud had a problem. Try again in a moment.',
  PROJECT_DELETED: 'That project no longer exists in iii cloud.',
  ENVIRONMENT_DELETED: 'That environment no longer exists in iii cloud.',
  NO_COMMITS: 'Commit something first: this folder has no commits yet, so there is nothing to put in iii cloud.',
  WRONG_WORKTREE:
    "That folder isn't the separate copy of your code this project made. Open the right folder, or choose Change project.",
  NEEDS_BROWSER_SIGN_IN: "This needs the browser sign-in: an API key can't do it. Sign in with the browser.",
  SIGN_IN_REQUIRED: 'You need to sign in to iii cloud again.',
  ENV_NAME_TAKEN: 'An environment with that name already exists in this project. Pick another name.',
  PROJECT_NAME_TAKEN: 'A project with that name already exists in this team. Pick another name.',
  BRANCH_NOT_FOUND: "That branch isn't on GitHub yet. Push it first, or pick another branch.",
  MANIFEST_NOT_FOUND: "That compose file isn't in the repository on that branch.",
  TEAM_FORBIDDEN: 'Only an owner or admin of the team can create this.',
  PLAN_LIMIT: 'The team plan has no room for another one. Remove one or change the plan.',
  NOT_CONNECTED: 'You are not signed in to iii cloud. Sign in again.',
  KEY_REJECTED: 'iii cloud rejected the API key. Check the key and try again.',
  NOT_FOUND: "iii cloud can't find that: it may have been deleted, or you can't see it.",
  ENVIRONMENT_CHANGED: 'Another environment was selected while you chose. Nothing was saved: check the values again.',
  ROLE_REQUIRED: "Changing this environment's values takes Owner or Admin. Nothing was saved.",
  INVALID_REQUEST:
    "Nothing was saved: one of the values can't be taken. Check the names and the values, then save again.",
}
const PLAIN_DETAIL: Record<string, string> = {
  BROWSER_SIGN_IN_UNAVAILABLE: 'Use an API key below. Browser sign-in arrives with the next iii cloud release.',
  MERGE_IN_PROGRESS: 'Your agent can finish it in its copy, then check again. Nothing was checked.',
  MERGE_CONFLICT:
    'Your agent can merge them in its copy and resolve the conflicts, then check again. Nothing was checked.',
  PUSH_REFUSED: 'Nothing was deployed. Check that you can push to this repository, then try again.',
  PUSH_UNKNOWN: 'See Deployments before you deploy again.',
}
const GENERIC = 'Something went wrong. Try again in a moment.'
/** Codes whose worker message is already the plain sentence ("billing no longer exists in iii cloud."). */
const OWN_WORDS = new Set([
  'PROJECT_DELETED',
  'MERGE_IN_PROGRESS',
  'WORKTREE_PATH_TAKEN',
  'NO_ACCESS',
  'OWN_BRANCH_TAKEN',
  'BRANCH_NOT_MOVED',
])
/**
 * `values::save`, per value: what happened to it, after "Not saved." (`LOCAL_NOT_SAVED` comes with a saved one:
 * iii cloud has it, the run here's .env does not). `{env}`, `{name}` and `{file}` are the row's.
 */
const VALUE_PLAIN: Record<string, string> = {
  NOT_TRIED: 'The save stopped before this one.',
  ALREADY_SET: "It's already set in {env}; a copy never replaces a value.",
  FILE_CHANGED: '{file} no longer has {name}. Pick another source, or add it back to the file.',
  NO_URL: '{env} has no address yet.',
  NOT_SAVED: "iii cloud didn't take it.",
  LOCAL_NOT_SAVED:
    "Saved in iii cloud. The .env of the local run couldn't be written, so the local run doesn't have it.",
}
/** One value's result in words. The worker's NOT_SAVED ("iii cloud refused it (CODE).") is kept: it is its own. */
export function valueMessage(e: { code: string; message: string }, row: { env: string; name: string; file?: string }) {
  if (e.code === 'NOT_SAVED' && /^iii cloud refused it \([A-Z0-9_]+\)\.$/.test(e.message)) return e.message
  const t = VALUE_PLAIN[e.code]
  if (!t) return plainMessage(e)
  return t
    .replace('{env}', row.env)
    .replace('{name}', row.name)
    .replace('{file}', row.file ?? 'The file')
}
/** A plain sentence for people: known codes are worded here; raw tool output never reaches the page. */
export const plainMessage = (e: { code: string; message: string }) => {
  const m = e.message.replace(new RegExp(`^(?:${e.code}|[A-Z][A-Z0-9_]{3,}):\\s*`), '')
  const raw = /\n|stderr|\bgh:|\bfatal:|\bexit (?:status|code)\b|^\s*at\s/i.test(m) || m.length > 200 || !m.trim()
  if (PLAIN[e.code] && (raw || !OWN_WORDS.has(e.code))) return PLAIN[e.code]
  return raw ? GENERIC : m
}
/** The files a `MERGE_CONFLICT` lists at the end of its message. */
export const conflictFiles = (message: string) => message.match(/Conflicting files: (.+?)\.?$/)?.[1].split(', ') ?? []
/** The faint line under a headline: a known explanation. The worker's `fix` is for the agent, never shown. */
export const plainDetail = (e: { code: string }) => PLAIN_DETAIL[e.code]

/** Problem ids the agent is working on, only while it works. */
export const workingOnIds = (s: Status): Set<string> =>
  new Set(s.agent?.state === 'working' ? (s.agent.working_on ?? []) : [])

/** The cloud part of the check did not run (the commit is not pushed, or this cloud does not plan it). */
export const cloudPending = (c: Check) =>
  c.plan.status === 'not_pushed' || c.plan.status === 'commit_not_planned' || c.plan.status === 'unavailable'

/**
 * The worktree's fix branch, which `check {push: true}` pushes first: an `iii-cloud/` branch no environment
 * deploys (the worker refuses any other). Null in the checkout, or on a branch an environment deploys.
 */
export function fixBranch(s: Status): string | null {
  const p = s.project
  const w = p?.workspace
  if (w?.mode !== 'worktree' || !w.branch.startsWith('iii-cloud/')) return null
  const envs = [...(p?.environments ?? []), ...(p?.cloud_projects ?? []).flatMap((x) => x.environments ?? [])]
  return envs.some((e) => e.branch === w.branch) ? null : w.branch
}

export function cloudPendingNote(s: Status): string | null {
  const c = s.check
  if (!c) return null
  const notOnGitHub = c.plan.status === 'not_pushed' || c.plan.status === 'commit_not_planned'
  // No open hand-off: no agent pushes it; the page's Check again does (CheckStage sends `push` with a fix branch).
  if (notOnGitHub && !(s.agent && !['done', 'stopped'].includes(s.agent.state))) {
    const b = fixBranch(s)
    return `iii cloud checks a commit once it is on GitHub${b ? `: Check again pushes ${b} first.` : '.'}`
  }
  if (c.plan.status === 'not_pushed') {
    return "iii cloud hasn't checked this commit yet: your agent pushes the branch, then checks again."
  }
  if (c.plan.status === 'commit_not_planned') {
    return "iii cloud can't see this commit in the repository yet. Your agent pushes the branch, then checks again."
  }
  if (c.plan.status === 'unavailable') return "iii cloud couldn't check this commit just now. Check again in a moment."
  return null
}

/** The agent is already on this kind of hand-off: starting another would start a second session. */
export function activeSameKind(s: Status, kind: HandoffKind): boolean {
  const a = s.agent
  if (!a || !['starting', 'working', 'waiting_for_you', 'waiting'].includes(a.state)) return false
  return (a.kind ?? s.handoff_brief?.kind ?? 'ready') === kind
}

/** The pull request that counts: a merged one, or an open one for the commit that was checked. A closed one is no pull request. */
export function usablePr(s: Status): Status['pr'] {
  const p = s.pr
  if (!p) return null
  if (p.state === 'MERGED') return p
  return p.state === 'OPEN' && s.check && p.head_sha === s.check.commit ? p : null
}

/** The repository's default branch, as the worker reports it. */
export const defaultBranch = (s: Status): string | null => s.project?.default_branch ?? null

/**
 * One set of sentence-case words for a deployment's state, the cloud console's (never lower-cased or
 * de-underscored on the fly). `ready` is the one serving (`Live`, see `deployState`); an older one is `Replaced`.
 */
export const OUTCOME_LABEL: Record<Deploy['outcome'], string> = {
  running: 'Deploying',
  ready: 'Live',
  failed: 'Failed',
  no_route: 'No route',
  canceled: 'Canceled',
  replaced: 'Replaced',
}
export const CONNECTION_LABEL: Record<Connection['status'], string> = {
  signed_out: 'Signed out',
  waiting: 'Waiting for you',
  connected: 'Signed in',
  expired: 'Expired',
  error: 'Error',
}
export const GONE_LABEL = 'Environment or deployment no longer exists'

export type Tone = RowTone | 'ghost'
type Stateful = { id: string; status: string; outcome?: Deploy['outcome']; current?: boolean }

/**
 * The word and dot of one deployment, the same on every screen (local record or iii cloud's list). `serving` is
 * the id of the one serving the environment (`servingId`): it is Live, and any other that went ready is Replaced.
 */
export function deployState(d: Stateful, serving: string | null = null): { label: string; tone: Tone } {
  if (d.status === 'gone') return { label: GONE_LABEL, tone: 'ghost' }
  if (d.current || d.id === serving) return { label: 'Live', tone: 'ok' }
  if (d.status === 'needs_action') return { label: 'Needs your action', tone: 'warn' }
  const o =
    d.outcome ?? (d.status === 'queued' || d.status === 'in_progress' ? 'running' : (d.status as Deploy['outcome']))
  const label = o === 'ready' ? OUTCOME_LABEL.replaced : (OUTCOME_LABEL[o] ?? o)
  return { label, tone: o === 'running' ? 'live' : o === 'failed' || o === 'no_route' ? 'alert' : 'ghost' }
}

/**
 * The id of the deployment serving an environment: iii cloud's `current` one. With the list read and no current row in
 * it (a rollback to a deployment beyond its first page) none is named; only without the list does the worker's own
 * newest ready one stand in.
 */
export const servingId = (s: Status, envId: string | undefined, cloud?: CloudDeployment[] | null): string | null =>
  cloud ? (cloud.find((d) => d.current)?.id ?? null) : (servingDeploy(s, envId)?.id ?? null)

/**
 * Whether iii cloud's list, read, may be hiding what serves the environment: it has a next page, or shows that
 * something was ready (a `ready` row; a `replaced` one never served: iii cloud replaces only a deployment that was
 * waiting for confirmation). Otherwise nothing is live: say so.
 */
export const listMayHideServing = (l: DeploymentList): boolean =>
  !!l.next_cursor || l.deployments.some((d) => d.status === 'ready')

/**
 * The deployment serving an environment, by iii cloud's list, for text that names it ("dev serves #5"). Until the
 * list is read it names none: the worker's own record can be out of date (a rollback made in the console).
 */
export function servingNamed(
  s: Status,
  envId: string | undefined,
  cloud: CloudDeployment[] | null | undefined,
): { number: number; sha: string } | null {
  if (!cloud) return null
  const id = servingId(s, envId, cloud)
  const c = cloud.find((d) => d.id === id)
  const l = s.deploys.find((d) => d.id === id)
  const number = c?.number ?? l?.number
  const sha = l?.commit.sha ?? c?.commit.sha
  return id && number != null && sha ? { number, sha } : null
}

/** The word of a local deployment record; pass `serving` to tell Live from Replaced. */
export const deployWord = (d: Deploy, serving: string | null = null) => deployState(d, serving).label

export type Live = { label: string; tone: Tone; sha: string | null }
/**
 * An environment's state from iii cloud's deployments (newest first); null before they are read. `nextCursor`: the
 * list's next page, if it has one (a first page of waiting deployments can hide what serves).
 */
export function liveOf(list: CloudDeployment[] | null | undefined, nextCursor?: string | null): Live | null {
  if (!list) return null
  const newest = list[0]
  const current = list.find((d) => d.current)
  if (newest && newest !== current && ['queued', 'in_progress', 'needs_action', 'failed'].includes(newest.status)) {
    const st = deployState(newest)
    return {
      label: newest.status === 'failed' ? 'Last deployment failed' : st.label,
      tone: st.tone,
      sha: newest.commit.sha,
    }
  }
  if (current) return { label: 'Live', tone: 'ok', sha: current.commit.sha }
  // No current row, but something may be live (a rollback to a deployment beyond the list's first page): the one rule
  // of `listMayHideServing`, so this and the footer cannot disagree; it is live, and the list can't say which.
  if (listMayHideServing({ deployments: list, next_cursor: nextCursor }))
    return { label: 'Live', tone: 'ok', sha: null }
  return { label: 'Nothing live yet', tone: 'ghost', sha: null }
}

/** 3 Dec: a short date in English whatever the browser's language. */
export const shortDate = (iso: string) => new Date(iso).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })

/** 14:34 today, else 3 Dec. */
export function when(iso: string): string {
  return new Date(iso).toDateString() === new Date().toDateString() ? clock(iso) : shortDate(iso)
}

/** Who started a deployment, as iii cloud says: you, a teammate by name, an API key, or null for a push. */
function whoStarted(cloud: CloudDeployment | null | undefined, s: Status, members: Members | null | undefined) {
  const uid = cloud?.triggered_by.user_id
  if (uid) {
    if (uid === s.connection.user?.id) return 'you'
    const u = members?.members.find((m) => m.user.id === uid)?.user
    return u?.name ?? u?.github_login ?? u?.email ?? 'a teammate'
  }
  return cloud?.triggered_by.api_key_id ? 'an API key' : null
}

export type Origin = {
  /** Who started it, in words ("you", "bruno"); null when no one is named (a push). */
  by: string | null
  /** Where it came from. */
  from: 'machine' | 'push' | 'console' | 'merge'
  /** A ready check of the deployed commit existed. */
  checked: boolean
  /** The pull request that was merged, for `from: 'merge'`. */
  pr: number | null
  /** "you, from this page", "a push to GitHub", "you, from the merge of #12 · approved by you". */
  origin: string
  /** `origin`, then ` · checked` or `, not checked`: "you, from this page · checked". */
  line: string
}

/**
 * The one answer to "who started this deployment, from where, and was it checked first".
 * The worker's own record (`status.deploys`) wins for where and whether it was checked: only it knows what it
 * deployed (`check_id`), merged (`pr`, `approved_by_worker`) and checked (`checked_ready`). iii cloud's list wins
 * for who: its `triggered_by` is the actor. `from_console` is not "from the console": the worker also sets it
 * for a push (it did not create that deployment either), so a push is told by `type` before it.
 */
export function deployOrigin(
  s: Status,
  ref: { id: string; type?: string },
  cloud?: CloudDeployment | null,
  members?: Members | null,
): Origin {
  const local = s.deploys.find((d) => d.id === ref.id)
  const type = local?.type ?? cloud?.type ?? ref.type
  const pr = local?.pr ?? null
  // `check_id` is set only by the worker's own deploy; an approval it made, or a record it created, say the same.
  const machine = !!local && (!!local.check_id || local.approved_by_worker || local.from_console === false)
  const from: Origin['from'] = pr != null ? 'merge' : machine ? 'machine' : type === 'push' ? 'push' : 'console'
  const named = whoStarted(cloud, s, members)
  // The worker deployed it with the person's credentials: an API key or no name from iii cloud is still "you".
  const by =
    from === 'machine' || (from === 'merge' && local?.approved_by_worker)
      ? named && named !== 'an API key'
        ? named
        : 'you'
      : named
  const what =
    from === 'merge'
      ? `the merge of #${pr}`
      : from === 'machine'
        ? 'this page'
        : from === 'push'
          ? 'a push to GitHub'
          : from === 'console'
            ? 'the console'
            : ''
  const origin = from === 'push' ? (by ? `${what} by ${by}` : what) : `${by ? `${by}, ` : ''}from ${what}`
  const checked = !!local?.checked_ready
  return { by, from, checked, pr, origin, line: `${origin}${checked ? ' · checked' : ', not checked'}` }
}

/** The agent is mid-run on another kind of hand-off: a new request goes to it as a follow-up, not into a new chat. */
export function followUp(s: Status, kind: HandoffKind): boolean {
  const a = s.agent
  if (!a || !['starting', 'working'].includes(a.state)) return false
  return (a.kind ?? s.handoff_brief?.kind ?? 'ready') !== kind
}
export const sendLabel = (s: Status, kind: HandoffKind, normal: string) =>
  followUp(s, kind) ? 'Send to your agent' : normal

/** A page of an environment in the iii cloud console (default: the selected one): '' is its overview, else `config`, `deployments`, `deployments/<n>`, `new-environment`. */
export function envPageUrl(s: Status, page: string, env?: string): string | null {
  const p = s.project
  const name = env ?? p?.environment?.name
  if (!p?.team?.slug || !p.cloud_project?.slug || !name) return null
  return `${s.connection.cloud_url.replace(/\/$/, '')}/${p.team.slug}/${p.cloud_project.slug}/${name}${page ? `/${page}` : ''}`
}
/** Where a team's deleted projects and environments are listed in the iii cloud console; null when the team is not known. */
export function deletionsUrl(s: Status, teamSlug: string | null | undefined): string | null {
  return teamSlug ? `${s.connection.cloud_url.replace(/\/$/, '')}/${teamSlug}/deletions` : null
}

/**
 * O1: a console setting in short words. `web` from `serve`, `container`, `port`, `api.prefix` and `access`
 * ("serves web on 3000 · team only"); any other setting as compact JSON.
 */
export function settingWords(setting: string, v: unknown): string {
  if (v == null) return 'not set'
  if (setting !== 'web' || typeof v !== 'object') return JSON.stringify(v)
  const w = v as {
    serve?: string
    container?: string
    port?: number
    directory?: string
    access?: string
    api?: { prefix?: string | null }
  }
  const api = w.api && 'prefix' in w.api ? (w.api.prefix ? `API under ${w.api.prefix}` : 'no API prefix') : null
  return [
    w.serve === 'static'
      ? `serves static files from ${w.directory ?? 'the build'}`
      : `serves ${w.container ?? 'the detected container'}${w.port ? ` on ${w.port}` : ''}`,
    api,
    w.access === 'team' ? 'team only' : w.access === 'public' ? 'public' : null,
  ]
    .filter(Boolean)
    .join(' · ')
}
/** O1: the file opens to the public what the console keeps to the team. */
export const opensToPublic = (o: { console: unknown; file: unknown }) =>
  (o.console as { access?: string } | null)?.access === 'team' &&
  (o.file as { access?: string } | null)?.access === 'public'

/** O2: the local config folder a LOCAL_CONFIG_NOT_SET problem names (its excerpt's `directory:`), else null. */
export function configDir(p: Problem | null | undefined): string | null {
  const line = p?.excerpt?.lines.find((l) => /^\s*directory:/.test(l))
  return (
    line
      ?.replace(/^\s*directory:\s*/, '')
      .replace(/^['"]|['"]$/g, '')
      .trim() || null
  )
}

/** The check says iii cloud wants a fresh sign-in even though the connection looks alive. */
export const signInRequired = (s: Status) => !!s.check?.problems.some((p) => p.code === 'SIGN_IN_REQUIRED')

export const needsAction = (d: Deploy) => d.status === 'needs_action'

/** What an environment serves: its newest ready deploy among the ones the worker knows (the selected env by default). */
export function servingDeploy(s: Status, envId = s.project?.environment?.id): Deploy | null {
  return s.deploys.find((d) => d.environment_id === envId && d.outcome === 'ready') ?? null
}

/** The branch an environment's assisted deploys push to (`project::own_branch`). */
export const ownBranch = (name: string) => `iii-cloud/${name}`

/**
 * How an environment gets a deploy, from production alone (`project::deploy_way`; both read
 * `tests/fixtures/deploy-way.json`). `own` is the branch an assisted environment moves to on its first deploy:
 * its branch is the default one, one another environment follows, or the default is unknown. `taken`: another
 * environment follows the branch it would push, so nothing is pushed. `envs` is the
 * project's environments, this one included.
 */
export type DeployWay = { way: 'pull_request' } | { way: 'assisted'; own: string | null; taken: boolean }
export function deployWay(
  base: string | null,
  env: { name: string; production: boolean; branch: string },
  envs: { name: string; branch: string }[],
): DeployWay {
  if (env.production) return { way: 'pull_request' }
  const own = ownBranch(env.name)
  const shared = envs.some((e) => e.name !== env.name && e.branch === env.branch)
  const moves = env.branch !== own && (base === null || base === env.branch || shared)
  const target = moves ? own : env.branch
  const taken = envs.some((e) => e.name !== env.name && e.branch === target)
  return { way: 'assisted', own: moves ? own : null, taken }
}

/** How the environment gets a deploy, decided from production alone (never from the hand-off's brief). */
export const wayOf = (s: Status, env: { name: string; production: boolean; branch: string }): DeployWay =>
  deployWay(defaultBranch(s), env, s.project?.environments ?? [])

export const ASSISTED = 'Assisted deploy'
/** The one sentence that says what an assisted deploy is. */
export const assistedHow = (env: string) =>
  `This page checks a commit in a clean copy of your code (what is committed, plus your agent's fixes) and, when you confirm, sends that commit to ${env}'s own branch, where iii cloud deploys it.`

/** The head of an environment's branch on GitHub, as the last refresh read it (`null` when unknown). */
export function envHead(s: Status, envId = s.project?.environment?.id): string | null {
  for (const p of s.project?.cloud_projects ?? []) {
    const e = p.environments?.find((x) => x.id === envId)
    if (e) return e.head ?? null
  }
  return null
}

/** The person's role on the project's team (`Owner`, `Admin`, `Developer`…), as iii cloud reports it. */
export function roleOf(s: Status): string | null {
  const teams = s.connection.teams
  return (teams.find((t) => t.id === s.project?.team?.id) ?? teams[0])?.role ?? null
}

export type RowTone = 'ok' | 'alert' | 'live' | 'edge' | 'warn'
export function stageTone(status: string): RowTone {
  if (/pass|ready|succe|complete|done|ok/i.test(status)) return 'ok'
  if (/fail|error/i.test(status)) return 'alert'
  if (/run|progress|active/i.test(status)) return 'live'
  if (/action|wait/i.test(status)) return 'warn'
  return 'edge'
}

export type DeployRow = { key: string; label: string; detail: string | null; time: string; tone: RowTone }
/** The six cloud stages plus the derived release row (after config and secrets). */
export function deployRows(d: Deploy): DeployRow[] {
  const failed = d.outcome === 'failed' || d.outcome === 'no_route'
  const rows: DeployRow[] = d.stages.map((s) => {
    const tone = stageTone(s.status)
    const ms = s.started_at && s.finished_at ? Date.parse(s.finished_at) - Date.parse(s.started_at) : null
    return {
      key: s.name,
      label: cloudStageLabel(s.name),
      detail: s.detail,
      tone,
      time:
        tone === 'live' ? 'live' : tone === 'edge' ? (failed ? 'not reached' : 'next') : ms != null ? duration(ms) : '',
    }
  })
  if (d.release_row) {
    const at = rows.findIndex((r) => r.key === 'config_secrets')
    const tone = stageTone(d.release_row.status)
    rows.splice(at + 1, 0, {
      key: 'release',
      label: 'Release command',
      detail: null,
      tone,
      time: tone === 'live' ? 'live' : d.release_row.duration_ms != null ? duration(d.release_row.duration_ms) : '',
    })
  }
  return rows
}

/** The canvas's five stages: the cloud's seven steps (config, secrets and the release command share one). */
const STAGE_GROUPS: [key: string, short: string, names: string[]][] = [
  ['plan', 'Plan', ['validation']],
  ['build', 'Build', ['build']],
  ['data', 'Database and secrets', ['resources', 'config_secrets', 'release']],
  ['workers', 'Workers', ['workers']],
  ['routes', 'Address', ['routes_tls']],
]

export type StageRow = { key: string; short: string; tone: RowTone; detail: string | null; time: string }
/** A deployment's stages as the canvas has them (`Plan · Build · Database and secrets · Workers · Address`). */
export function stageRows(d: Deploy): StageRow[] {
  const failed = isFailedDeploy(d)
  const steps = [
    ...d.stages.map((s) => ({
      name: s.name,
      tone: stageTone(s.status),
      detail: s.detail,
      ms: s.started_at && s.finished_at ? Date.parse(s.finished_at) - Date.parse(s.started_at) : null,
    })),
    ...(d.release_row
      ? [
          {
            name: 'release',
            tone: stageTone(d.release_row.status),
            detail: null,
            ms: d.release_row.duration_ms,
          },
        ]
      : []),
  ]
  const known = new Set(STAGE_GROUPS.flatMap(([, , names]) => names))
  const groups = [
    ...STAGE_GROUPS,
    ...steps
      .filter((x) => !known.has(x.name))
      .map((x): (typeof STAGE_GROUPS)[number] => [x.name, cloudStageLabel(x.name), [x.name]]),
  ]
  return groups.flatMap(([key, short, names]) => {
    const part = steps.filter((x) => names.includes(x.name))
    if (!part.length) return []
    const tones = part.map((x) => x.tone)
    const tone: RowTone = tones.includes('alert')
      ? 'alert'
      : tones.includes('live')
        ? 'live'
        : tones.every((t) => t === 'ok')
          ? 'ok'
          : tones.includes('warn')
            ? 'warn'
            : tones.includes('ok') && !failed
              ? 'live'
              : 'edge'
    const detail = part.find((x) => x.detail)?.detail ?? null
    const ms = part.reduce((sum, x) => sum + (x.ms ?? 0), 0)
    const took = part.some((x) => x.ms != null) ? duration(ms) : null
    const time =
      tone === 'live'
        ? 'running'
        : tone === 'edge'
          ? failed
            ? 'not run'
            : 'next'
          : [detail, tone === 'alert' ? null : took].filter(Boolean).join(' · ')
    return [{ key, short, tone, detail, time }]
  })
}

const FAILURE_WORDS: Record<string, string> = {
  OOMKilled: 'ran out of memory',
  CrashLoopBackOff: 'keeps crashing',
  ImagePullBackOff: "its image can't be pulled",
  ErrImagePull: "its image can't be pulled",
  NO_ROUTE: 'serves no route',
  no_route: 'serves no route',
  HEALTHCHECK_FAILED: "the healthcheck doesn't answer",
}
/** "registry ran out of memory": the worker and what happened to it, in words. */
export const failureWords = (e: { worker: string | null; code: string }) =>
  `${e.worker ? `${e.worker} ` : ''}${FAILURE_WORDS[e.code] ?? `failed (${e.code})`}`

export const failedStage = (d: Deploy) => (d.error ? cloudStageLabel(d.error.stage) : 'a stage')

export const isFailedDeploy = (d: Deploy) => d.outcome === 'failed' || d.outcome === 'no_route'

/** A deploy of the environment is still going: what the check and the review say instead of deploying over it. */
export const deployBusy = (s: Status, d: Deploy, env: string) => ({
  headline: `Deployment #${d.number} to ${env} is already running`,
  detail: `It started ${deployOrigin(s, d).origin} ${ago(d.created_at)}. Starting yours now would replace it.`,
})

/** A bare code is not a title: "LOCK_MISSING" reads "Lock missing". */
export function problemTitle(p: Problem): string {
  if (p.title && p.title !== p.code) return p.title
  if (p.code === 'DATABASE_NOT_MANAGED') return 'The database should come from iii cloud'
  const words = p.code.toLowerCase().replace(/_/g, ' ')
  return words.charAt(0).toUpperCase() + words.slice(1)
}

/** A cloud plan's blocking codes named in a hint ("…issues: LOCK_MISSING, INVALID_COMPOSE_FILE."), in the page's words. */
export function planCodeWords(s: Status, hint: string | null): string[] {
  const codes = hint?.match(/issues?:\s*([A-Z][A-Z0-9_]*(?:,\s*[A-Z][A-Z0-9_]*)*)/)?.[1].split(/,\s*/) ?? []
  return codes.map((code) => {
    const p = s.check?.problems.find((x) => x.code === code)
    return p ? problemTitle(p) : problemTitle({ code, title: code } as Problem)
  })
}

/** The deployment row a cloud stage belongs to (`validation` is `plan`). */
export const stageKeyOf = (name: string) => STAGE_GROUPS.find(([, , names]) => names.includes(name))?.[0] ?? name

/** The cloud's message without the `CODE · ` prefix older checks carried. */
export function problemDetail(p: Problem): string | null {
  const d = p.detail?.startsWith(`${p.code} · `) ? p.detail.slice(p.code.length + 3) : p.detail
  return d || null
}

/** A person-owned variable the page never asks for: a database URL (iii cloud provides it) or one noted as not needed. */
export function hiddenPerson(s: Status, p: Problem): boolean {
  if (p.owner !== 'person' || p.code !== 'LOCAL_VARIABLE_UNSET') return false
  const name = variableName(p)
  if (!name) return false
  // (unless the person chose a database outside iii cloud: then the request is flagged `external`)
  if (isDatabaseVar(name) && !s.secrets.some((r) => r.name === name && r.external)) return true
  return s.secrets.some((r) => r.name === name && r.mode === 'not_needed')
}

/** The codes about a variable or a secret of the environment: set where they are listed (Secrets and variables), never rows of the check. */
export const VALUE_CODES = new Set(['EMPTY_DEFAULT_VARIABLE', 'SECRET_MISSING', 'LOCAL_VARIABLE_UNSET'])

/**
 * The one list every count comes from: the current check's problems that still need someone.
 * Person warnings (decisions with defaults) stop counting once the check is ready.
 */
export function openProblems(s: Status): Problem[] {
  const c = s.check
  if (!c) return []
  // A name skipped for now never counts, unless iii cloud still blocks on it (SECRET_MISSING).
  return c.problems.filter(
    (p) =>
      p.owner !== 'none' &&
      !VALUE_CODES.has(p.code) &&
      !hiddenPerson(s, p) &&
      !(p.skipped && p.severity !== 'block') &&
      !(c.ready && p.owner === 'person' && p.severity !== 'block'),
  )
}

/** The name a LOCAL_VARIABLE_UNSET problem asks a value for, from its title or detail. */
export function variableName(p: { title: string; detail: string | null; code: string | null }): string | null {
  const m = `${p.title} ${p.detail ?? ''}`.match(/\b([A-Z_][A-Z0-9_]{2,})\b/)
  return m && m[1] !== p.code ? m[1] : null
}

/** Where the details live: the person's chat message stays one plain sentence. */
export const HANDOFF_POINTER = 'Details: iii-cloud::handoff::brief · skill iii://iii-cloud'

/** "1 commit", "3 commits"; `many` for an irregular plural. */
export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`

/** What the person says to the agent: a plain sentence. The problems, fixes and paths are in `status`. */
export function handoffMessage(s: Status): string {
  const n = openProblems(s).length
  const what = envName(s) || s.project?.repository || 'this project'
  return n
    ? `Make ${what} ready for iii cloud: fix the ${plural(n, 'problem', 'problems')} from the latest check.`
    : `Make ${what} ready for iii cloud.`
}

export type ImportChoice = { path: string; environment: string | null }

export function importMessage(s: Status, choice?: ImportChoice): string {
  const p = s.project
  const name = p?.repository ?? 'this repository'
  if (choice) return `Import ${name} with ${choice.path}${choice.environment ? ` as ${choice.environment}` : ''}.`
  return `Import ${name} into iii cloud${p?.team ? ` (team ${p.team.name})` : ''}.`
}

/** The compose file an environment checks and deploys: its manifest, else the root file. */
export const composeFile = (manifest: string | null | undefined) => manifest || 'worker-compose.yaml'

/** The compose file: the latest check's, else the environment's. `switches`: it is not the environment's,
    so the environment moves to it when the person confirms the deploy. */
export function appFile(s: Status): { path: string; switches: boolean; note: string | null } {
  const current = composeFile(s.project?.environment?.manifest)
  const path = s.check?.manifest || current
  const switches = path !== current
  return {
    path,
    switches,
    note: switches
      ? `${s.project?.environment?.name ?? 'The environment'} switches to it when you confirm the deploy`
      : null,
  }
}

/** A page check names its file (the page's own), so it can also go back to the environment's. */
export function withAppFile(s: Status, payload: Record<string, unknown> = {}): Record<string, unknown> {
  return { ...payload, manifest: appFile(s).path }
}

/** Why the picker can't take a file. */
export const COMPOSE_ISSUE: Record<string, string> = {
  NOT_WORKER_COMPOSE: 'not a worker-compose*.yaml',
}

export function pullRequestMessage(s: Status): string {
  return `${envName(s) || 'This project'} is ready: open the pull request for it.`
}

export function fixMessage(d: Deploy, s: Status): string {
  return `The deploy ${sha7(d.commit.sha)} of ${envName(s) || 'this environment'} failed at ${failedStage(d)}. Fix it.`
}

/** A function result as the card gets it: the `details` of the harness envelope, null on an error. */
export function unwrapDetails(output: unknown): Record<string, unknown> | null {
  let v = output
  if (typeof v === 'string') {
    try {
      v = JSON.parse(v)
    } catch {
      return null
    }
  }
  if (!v || typeof v !== 'object') return null
  if ('error' in v && (v as { error?: unknown }).error) return null
  if ((v as { isError?: unknown }).isError) return null
  const d = unwrapEnvelope(v)
  return d && typeof d === 'object' && !Array.isArray(d) ? (d as Record<string, unknown>) : null
}

export const workerError = (err: unknown, code: string | undefined, message: string): WorkerError => {
  const e = err as { fix?: unknown; started_at?: unknown; run_id?: unknown } | null
  // `RUN_BUSY` puts the run id and its start time in the message: read them when they are not fields.
  const startedInMessage = message.match(/\d{4}-\d{2}-\d{2}T[\d:.]+(?:Z|[+-]\d{2}:?\d{2})?/)?.[0]
  const runInMessage = message.match(/run[ _-]?(?:id)?[ :=]+([\w-]{6,})/i)?.[1]
  return {
    code: code ?? 'ERROR',
    message,
    fix: typeof e?.fix === 'string' ? e.fix : undefined,
    started_at: typeof e?.started_at === 'string' ? e.started_at : code === 'RUN_BUSY' ? startedInMessage : undefined,
    run_id: typeof e?.run_id === 'string' ? e.run_id : code === 'RUN_BUSY' ? runInMessage : undefined,
  }
}

/** The header description: who is signed in, and for which team or key. */
export function whoLine(s: Status): string {
  const c = s.connection
  if (!isConnected(c)) return 'Not connected'
  const id = s.project?.team?.id ?? (c.api_key ? c.api_key.team_id : c.teams[0]?.id)
  const team = c.teams.find((t) => t.id === id) ?? c.teams[0]
  const name = s.project?.team?.name ?? team?.name
  if (c.api_key) return `API key${name ? ` · ${name}` : ''}`
  const role = team?.role ? ` (${team.role.charAt(0).toUpperCase()}${team.role.slice(1)})` : ''
  const who = c.user?.name || c.user?.email || 'Signed in'
  return `${who}${name ? ` · team ${name}${role}` : ''}`
}

export function expiresIn(c: Connection, now = Date.now()): string {
  if (!c.expires_at) return ''
  const days = Math.round((Date.parse(c.expires_at) - now) / 86_400_000)
  return days > 0 ? `${days} days` : 'expires today'
}

/** A database connection string is never the person's to type: iii cloud provides the database. */
export const isDatabaseVar = (name: string) =>
  /(^|_)(DATABASE|POSTGRES|POSTGRESQL|PGHOST|PGDATABASE|DB)(_|$)/i.test(name)

/** The sentence about a Postgres the deploy creates, from the plan; null when the plan has none. */
export function postgresNote(s: Status): string | null {
  const has = s.check?.plan.confirmations.resources.some((r) => /postgres/i.test(r.kind))
  return has ? `iii cloud creates a Postgres database for ${envName(s)}. You confirm it when you deploy.` : null
}

/** Deploy refusals that mean the check no longer holds: the only way on is to check again. */
export const CHECK_AGAIN = new Set(['NOT_READY', 'MAIN_MOVED', 'PLAN_CHANGED', 'PR_HEAD_CHANGED', 'SOURCE_CHANGED'])

/** How a plan's resource kinds read. */
export const RESOURCE_KIND: Record<string, string> = { postgres: 'Postgres', redis: 'Redis', rabbitmq: 'RabbitMQ' }

/** The median time of an environment's deploys that went live; null before the first one. */
export function typicalDeployMs(s: Status, envId: string): number | null {
  const took = s.deploys
    .filter((d) => d.environment_id === envId && d.outcome === 'ready' && d.finished_at)
    .map((d) => Date.parse(d.finished_at as string) - Date.parse(d.created_at))
    .sort((a, b) => a - b)
  return took.length ? took[Math.floor(took.length / 2)] : null
}

/** "4 to 6": the spread, in whole minutes, of the environment's last five deploys that went live; null when they agree. */
export function recentDeployRange(s: Status, envId: string): string | null {
  const mins = s.deploys
    .filter((d) => d.environment_id === envId && d.outcome === 'ready' && d.finished_at)
    .slice(0, 5)
    .map((d) => Math.max(1, Math.round((Date.parse(d.finished_at as string) - Date.parse(d.created_at)) / 60_000)))
  const [lo, hi] = [Math.min(...mins), Math.max(...mins)]
  return mins.length > 1 && lo !== hi ? `${lo} to ${hi}` : null
}

export const minutesLabel = (ms: number) => {
  const min = Math.max(1, Math.round(ms / 60_000))
  return min < 60 ? `${min} min` : `${Math.floor(min / 60)} h ${min % 60} min`
}

export type CheckRow = {
  id: string
  /** The current problem's id, when it is still open. */
  pid: string | null
  title: string
  code: string | null
  owner: string
  fixed: boolean
  /** Where it is (the brief's, for a fixed row), to group and deduplicate rows. */
  file: string | null
  line: number | null
  detail: string | null
}

const briefKey = (title: string, file: string | null, line: number | null) => `${title}|${file ?? ''}|${line ?? ''}`

/**
 * "N of M fixed": the problems the hand-off was made for (from `status.handoff_brief`) marked fixed once
 * gone from the current check. Problems that appeared since follow. No brief: the plain current list.
 */
export function checklist(s: Status): { rows: CheckRow[]; fixed: number; counted: boolean } {
  const open = openProblems(s)
  const brief = s.handoff_brief?.kind === 'ready' ? s.handoff_brief : null
  if (!brief) {
    return {
      rows: open.map((p) => ({
        id: p.id,
        pid: p.id,
        title: problemTitle(p),
        code: p.code,
        owner: p.owner,
        fixed: false,
        file: p.file,
        line: p.line,
        detail: p.detail,
      })),
      fixed: 0,
      counted: false,
    }
  }
  const now = new Map(open.map((p) => [briefKey(problemTitle(p), p.file, p.line), p]))
  const rows: CheckRow[] = [
    ...brief.agent_problems.map((b) => ({ b, owner: 'agent' })),
    ...brief.person_problems.map((b) => ({ b, owner: 'person' })),
  ].map(({ b, owner }) => {
    const key = briefKey(b.title, b.file, b.line)
    const p = now.get(key)
    now.delete(key)
    return {
      id: key,
      pid: p?.id ?? null,
      title: b.title,
      code: p?.code ?? null,
      owner,
      fixed: !p,
      file: p?.file ?? b.file,
      line: p?.line ?? b.line,
      detail: p?.detail ?? b.detail,
    }
  })
  for (const [key, p] of now)
    rows.push({
      id: key,
      pid: p.id,
      title: problemTitle(p),
      code: p.code,
      owner: p.owner,
      fixed: false,
      file: p.file,
      line: p.line,
      detail: p.detail,
    })
  return { rows, fixed: rows.filter((r) => r.fixed).length, counted: true }
}

/** `/home/you/x` reads `~/x`. */
export const tilde = (path: string) => path.replace(/^\/(?:home|Users)\/[^/]+(?=\/|$)/, '~')

/** The cloud project's name, else the repository: what the person calls the selected project. */
export const projectName = (s: Status) => s.project?.cloud_project?.name ?? s.project?.repository ?? ''

/**
 * What waits on the person in the selected project, as clauses: the agent's question, a secret to enter, a
 * deploy of the selected environment that iii cloud holds. Leaving keeps them; the page just stops showing them.
 */
export function whatWaits(s: Status): string[] {
  const secrets = s.secrets.filter((r) => r.state === 'waiting').length
  const held = s.deploys.find((d) => d.environment_id === s.project?.environment?.id && needsAction(d))
  return [
    s.agent?.state === 'waiting_for_you' ? 'your agent has a question' : null,
    secrets ? `${plural(secrets, 'secret')} ${secrets === 1 ? 'is' : 'are'} waiting` : null,
    held ? `deploy #${held.number} needs your action` : null,
  ].filter((w): w is string => !!w)
}

import type { Host } from '@iii-dev/console-ui'
import { Button, Panel, Skeleton } from '@iii-dev/console-ui'
import { useCloud } from './api'
import { clock } from './asked'
import { failedLine } from './FailureCard'
import { needsYou } from './findings'
import { useActions, useLocked } from './hub'
import {
  deployOrigin,
  deployRows,
  fixBranch,
  isFailedDeploy,
  liveOf,
  minutesLabel,
  needsAction,
  plural,
  roleOf,
  servingDeploy,
  sha7,
  sourceLabel,
  type Tone,
  wayOf,
  when,
  withAppFile,
} from './model'
import { Dot, ErrorPanel, Mono, monoShas, SectionCard as Section, StatePill } from './parts'
import type { EnvTabProps, Route } from './route'
import type { CloudDeployment, DeploymentList, Members, Status, Values, WorkerError } from './types'

// The deployment types, the one-source functions and the Feeds context live in the foundation; re-exported
// so the pages keep importing them from here.
export { Feeds, useDeployments } from './api'
export { deployState, type Live, liveOf, when } from './model'
export type { CloudDeployment, DeploymentList, Members } from './types'

/** The shared pieces (parts.tsx) under the names the pages already use. */
export const StateChip = ({ live }: { live: { label: string; tone: Tone } }) => <StatePill state={live} />
export { Section }

export type Health = {
  status: 'healthy' | 'degraded' | 'down' | 'stopped'
  checked_at: string
  workers: { worker: string; ready: boolean }[]
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

// ---- the next-action card ----

/** What the card at the top of an environment says, one at a time. */
export type NextState =
  | 'held'
  | 'running'
  | 'you'
  | 'agent'
  | 'passed'
  | 'failed'
  | 'pull-request'
  | 'deploy'
  | 'empty'
  | 'current'

export type NextButton =
  /** Starts the check, then goes to the Deploy place. */
  | { label: string; kind: 'check' }
  | { label: string; kind: 'go'; to: Route }
  | { label: string; kind: 'link'; href: string }

export type Next = {
  state: NextState
  /** A strong outline: the card waits for the person. A plain one: they can leave. */
  wait: boolean
  /** The button is the primary one (the card waits, or something failed). */
  primary: boolean
  title: string
  detail: string
  action?: NextButton
}

const sameCommit = (a: string, b: string) => !!a && !!b && (a.startsWith(b) || b.startsWith(a))
const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1)

const stays = (n: number) => `${plural(n, 'uncommitted change')} ${n === 1 ? 'stays' : 'stay'} in your folder`

/**
 * The one thing to do next in the selected environment `envId`, the first that holds, in this order:
 *  1. held: a deployment iii cloud holds for the person's approval
 *  2. running: a deployment is going
 *  3. you: the latest check has problems only the person can solve
 *  4. agent: the agent is working, or the check is running, and nothing needs the person
 *  5. passed: a check passed on a commit the environment does not serve (not one whose deployment just failed)
 *  6. failed: the newest deployment failed (unless the folder has moved past that commit: then 9)
 *  7. pull-request: nothing is live, the environment deploys from a pull request, and one is open
 *  8. deploy: the folder has commits the environment does not serve
 *  9. empty / current: nothing is live yet, or it serves everything in the folder
 * `live` is what serves the environment (iii cloud's current deployment, else the worker's newest ready one).
 */
export function nextAction(
  s: Status,
  envId: string,
  opts: {
    live?: { id: string; sha: string } | null
    cloud?: CloudDeployment[] | null
    members?: Members | null
    now?: number
    /** A first deploy's values read: the count groups the names a file can fill. */
    values?: Values | null
    /** A first deploy whose values read has not arrived: no number is printed. */
    valuesPending?: boolean
  } = {},
): Next {
  const now = opts.now ?? Date.now()
  const p = s.project
  const env = p?.environments.find((e) => e.id === envId)
  const name = env?.name ?? 'this environment'
  const mine = s.deploys.filter((d) => d.environment_id === envId)
  const live = opts.live ?? null
  const liveNumber = live
    ? (mine.find((d) => d.id === live.id)?.number ?? opts.cloud?.find((d) => d.id === live.id)?.number)
    : null
  const serves = liveNumber ? `${name} serves #${liveNumber}` : null

  const held = mine.find(needsAction)
  if (held) {
    return {
      state: 'held',
      wait: true,
      primary: true,
      title: `Deployment #${held.number} needs your action`,
      detail: `iii cloud holds it until you approve what it would change.${serves ? ` ${serves} until then.` : ''}`,
      action: { label: 'Review it', kind: 'go', to: { at: 'deploy', env: envId, deploy: held.id } },
    }
  }
  const running = mine.find((d) => d.outcome === 'running')
  if (running) {
    const stage = deployRows(running).find((r) => r.tone === 'live')
    return {
      state: 'running',
      wait: false,
      primary: false,
      title: `Deployment #${running.number} is deploying`,
      detail: `${stage ? `${stage.label}${stage.detail ? `: ${stage.detail}` : ''} · ` : ''}${minutesLabel(now - Date.parse(running.created_at))} so far.${serves ? ` ${serves} until it's live.` : ''}`,
      action: { label: 'Follow', kind: 'go', to: { at: 'deploy', env: envId, deploy: running.id } },
    }
  }

  const c = s.check?.environment_id === envId ? s.check : null
  const branch = p ? sourceLabel(s, name) : null
  // The same list the Deploy screen's title counts.
  const you = c && !c.ready && c.run?.status !== 'running' ? needsYou(s, opts.values ?? null) : null
  const n = you?.count ?? 0
  const a = s.agent && ['starting', 'working', 'waiting'].includes(s.agent.state) ? s.agent : null
  const fixing = a?.working_on?.length ?? 0
  if (you && n) {
    const first =
      you.items[0]?.title ??
      (you.decide[0]
        ? `${name}'s ${you.decide[0].setting} was changed in iii cloud's console`
        : `Set ${plural(you.values.length, 'value')} for ${name}`)
    // A first deploy's count needs the values read (it groups what a .env can fill): without it, no number.
    const title = opts.valuesPending
      ? `Things need you before ${name} can get ${branch ?? 'your changes'}`
      : `${plural(n, 'thing')} ${n === 1 ? 'needs' : 'need'} you before ${name} can get ${branch ?? 'your changes'}`
    return {
      state: 'you',
      wait: true,
      primary: true,
      title,
      detail: `${first}${n > 1 && !opts.valuesPending ? ` and ${n - 1} more` : ''}.${a && fixing ? ` Your agent is fixing ${plural(fixing, 'problem')} meanwhile.` : ''}`,
      action: { label: 'Open the check', kind: 'go', to: { at: 'deploy', env: envId } },
    }
  }
  // The agent waits for an answer or is stuck: the check needs the person, and a new check would start in its copy.
  if (s.agent && ['waiting_for_you', 'stalled'].includes(s.agent.state)) {
    return {
      state: 'you',
      wait: true,
      primary: true,
      title: `The check for ${name} needs you`,
      detail:
        s.agent.state === 'waiting_for_you'
          ? (s.agent.question ?? 'Your agent has a question in its chat.')
          : (s.agent.question ??
            'Your agent made no progress for a while. Open the check to read what it tried, or stop it.'),
      action: { label: 'Open the check', kind: 'go', to: { at: 'deploy', env: envId } },
    }
  }
  if (a || c?.run?.status === 'running') {
    return {
      state: 'agent',
      wait: false,
      primary: false,
      title: a
        ? fixing
          ? `Your agent is fixing ${plural(fixing, 'problem')} for ${name}`
          : `Your agent is working on ${name}`
        : `A check of ${name} is running`,
      detail: a
        ? `Working ${minutesLabel(now - Date.parse(a.started_at))} in a separate copy. The check runs again when it commits.`
        : 'It runs on this machine. Open it to follow.',
      action: { label: 'Open the check', kind: 'go', to: { at: 'deploy', env: envId } },
    }
  }

  const newest = mine[0]
  const failed = newest && isFailedDeploy(newest) ? newest : null
  if (c?.ready && !(live && sameCommit(live.sha, c.commit)) && !(failed && sameCommit(failed.commit.sha, c.commit))) {
    const commits = c.commits?.length ?? 0
    const made = c.plan.confirmations.resources.length
    const facts = [commits ? plural(commits, 'commit') : null, made ? `${plural(made, 'new resource')}` : null]
    return {
      state: 'passed',
      wait: true,
      primary: true,
      title: `${sha7(c.commit)} passed the check at ${clock(c.checked_at)}`,
      detail: `${facts.filter(Boolean).join(', ')}${facts.some(Boolean) ? '. ' : ''}It waits for you.`,
      action: { label: 'Review and deploy', kind: 'go', to: { at: 'deploy', env: envId } },
    }
  }

  const files = p?.checkout.changed_files.length ?? 0
  const notServed = p?.checkout.not_served ?? []
  const folderMoved = notServed.length > 0 && !(failed && sameCommit(notServed[0].sha, failed.commit.sha))
  if (failed && !folderMoved) {
    const cloud = opts.cloud?.find((d) => d.id === failed.id)
    return {
      state: 'failed',
      wait: false,
      primary: true,
      title: failedLine(failed),
      detail: `${cap(deployOrigin(s, failed, cloud, opts.members).line)}.${liveNumber ? ` ${name} still serves #${liveNumber}.` : ` Nothing was live on ${name}.`}`,
      action: { label: 'See what failed', kind: 'go', to: { at: 'env', env: envId, deploy: failed.id } },
    }
  }

  if (!live && env?.production && s.pr?.state === 'OPEN') {
    const role = roleOf(s)
    const merges = role && /^(owner|admin)$/i.test(role)
    return {
      state: 'pull-request',
      wait: false,
      primary: false,
      title: `Nothing is live on ${name} yet`,
      detail: `It deploys when pull request #${s.pr.number} into ${s.pr.base} is merged.${merges ? " You're allowed to merge it." : role ? ` An Owner or Admin merges it; you're ${/^[aeiou]/i.test(role) ? 'an' : 'a'} ${role}.` : ''}`,
      action: { label: `Open pull request #${s.pr.number}`, kind: 'link', href: s.pr.url },
    }
  }

  if (notServed.length > 0) {
    const top = notServed[0]
    const rest = files ? stays(files) : 'nothing uncommitted in your folder'
    return {
      state: 'deploy',
      wait: true,
      primary: true,
      title: `${plural(notServed.length, 'commit')} on ${branch ?? 'your branch'} ${notServed.length === 1 ? "isn't" : "aren't"} on ${name}`,
      detail: `${sha7(top.sha)} ${top.message.split('\n')[0]} · ${rest}`,
      action: { label: `Deploy to ${name}`, kind: 'check' },
    }
  }

  if (!live) {
    return {
      state: 'empty',
      wait: false,
      primary: false,
      title: `Nothing is live on ${name} yet`,
      detail: env?.production
        ? `It deploys when a pull request into ${env.branch} is merged.`
        : 'An assisted deploy puts the first version live. It starts with the check.',
      action: { label: 'Open the check', kind: 'go', to: { at: 'deploy', env: envId } },
    }
  }
  return {
    state: 'current',
    wait: false,
    primary: false,
    title: `${name} serves everything in your folder`,
    detail: `${branch ?? 'Your branch'}${p ? ` at ${sha7(p.checkout.head)}` : ''}, ${files ? stays(files) : 'nothing uncommitted'}.`,
  }
}

/** The next-action card: the title, one sentence, and one action or none. */
export function NextCard({
  host,
  status,
  envId,
  next,
  go,
}: {
  host: Host
  status: Status
  envId: string
  next: Next
  go: (r: Route) => void
}) {
  const { call } = useActions(host)
  const locked = useLocked(host, status)
  const act = next.action
  const press = () => {
    if (act?.kind === 'go') return go(act.to)
    if (act?.kind !== 'check') return
    // The check runs on; the Deploy place shows it from the hub's `checking` count.
    void call(
      'check',
      'iii-cloud::check',
      withAppFile(status, { run: true, ...(fixBranch(status) ? { push: true } : {}) }),
    )
    go({ at: 'deploy', env: envId })
  }
  const variant = next.primary ? 'primary' : 'secondary'
  return (
    <section
      className="iii-cloud-next"
      aria-label="Next"
      data-state={next.state}
      data-wait={next.wait ? '' : undefined}
      data-failed={next.state === 'failed' ? '' : undefined}
    >
      <div className="iii-cloud-next-text">
        <span className="iii-cloud-next-label">Next</span>
        <h3 className="iii-cloud-next-title">{monoShas(next.title)}</h3>
        <p className="iii-cloud-desc">{monoShas(next.detail)}</p>
      </div>
      {act?.kind === 'link' ? (
        <Button variant={variant} asChild>
          <a href={act.href} target="_blank" rel="noreferrer">
            {act.label}
          </a>
        </Button>
      ) : act ? (
        <Button
          variant={variant}
          disabled={locked && (act.kind === 'check' || next.state === 'passed' || next.state === 'held')}
          onClick={press}
        >
          {act.label}
        </Button>
      ) : null}
    </section>
  )
}

// ---- Live now ----

/** What serves the environment: iii cloud's current deployment, else the worker's newest ready one. */
export function servingOf(s: Status, envId: string, cloud: CloudDeployment[] | null | undefined) {
  const c = cloud?.find((d) => d.current)
  if (c)
    return {
      id: c.id,
      number: c.number,
      sha: c.commit.sha,
      message: c.commit.message ?? null,
      since: c.finished_at ?? c.created_at,
    }
  const l = servingDeploy(s, envId)
  return l
    ? { id: l.id, number: l.number, sha: l.commit.sha, message: l.commit.message, since: l.finished_at ?? l.created_at }
    : null
}

/** Live now: one card of three cells: what serves, its health, and how a deployment starts. */
export function LiveNow({
  host,
  status,
  envId,
  feed,
  narrow,
}: Pick<EnvTabProps, 'host' | 'status' | 'envId'> & {
  feed: { data: DeploymentList | null; error: WorkerError | null }
  /** A phone: one compact card (chip, address, what serves, how many workers are up). */
  narrow?: boolean
}) {
  const p = status.project
  const env = p?.environments.find((e) => e.id === envId)
  const serving = servingOf(status, envId, feed.data?.deployments)
  const health = useCloud<Health>(host, status, envId, serving ? '/health' : null)
  if (!env) return null
  const h = health.data ? healthOf(health.data, env.name) : null
  const way = wayOf(status, env)
  // The two sentences of the "Deploys from" cell; a phone's card says them in one line.
  const words = {
    value: way.way === 'assisted' ? 'an assisted deploy' : 'a pull request',
    detail:
      way.way === 'pull_request'
        ? `when one into ${status.pr?.base ?? env.branch} is merged`
        : way.own
          ? `the first one moves ${env.name} from ${env.branch} to its own branch, ${way.own}`
          : `to ${env.branch}; a push to it from anywhere else deploys without the check`,
  }
  if (narrow) {
    const live = liveOf(feed.data?.deployments, feed.data?.next_cursor)
    const url = p?.environment?.id === envId ? p.environment.url : null
    return (
      <Panel className="iii-cloud-panel iii-cloud-livecard" role="region" aria-label="Live now">
        <span className="iii-cloud-row">
          {live ? <StatePill state={live} /> : null}
          {url ? (
            <a className="iii-cloud-mono iii-cloud-url iii-cloud-link" href={url} target="_blank" rel="noreferrer">
              {url.replace(/^https?:\/\//, '')}
            </a>
          ) : null}
        </span>
        {feed.error ? (
          <ErrorPanel error={feed.error} />
        ) : !feed.data && !serving ? (
          <Skeleton className="iii-cloud-skeleton-row" />
        ) : serving ? (
          <>
            <span className="iii-cloud-text">
              <Mono>{sha7(serving.sha)}</Mono> {serving.message?.split('\n')[0] ?? ''}
            </span>
            <span className="iii-cloud-desc">
              deployment #{serving.number}, since {when(serving.since)}
              {h ? ` · ${h.text}` : ''}
            </span>
          </>
        ) : (
          <span className="iii-cloud-text">Nothing is live yet</span>
        )}
        <span className="iii-cloud-desc">{`Deploys from ${words.value}, ${words.detail}`}</span>
      </Panel>
    )
  }
  return (
    <Panel className="iii-cloud-panel iii-cloud-live" role="region" aria-label="Live now">
      <div className="iii-cloud-cell">
        <span className="iii-cloud-cell-label">Serving</span>
        {feed.error ? (
          <ErrorPanel error={feed.error} />
        ) : !feed.data && !serving ? (
          <Skeleton className="iii-cloud-skeleton-row" />
        ) : serving ? (
          <>
            <span className="iii-cloud-cell-value">
              <Mono>{sha7(serving.sha)}</Mono> {serving.message?.split('\n')[0] ?? ''}
            </span>
            <span className="iii-cloud-desc">
              deployment #{serving.number}, since {when(serving.since)}
            </span>
          </>
        ) : (
          <span className="iii-cloud-cell-value">Nothing is live yet</span>
        )}
      </div>
      <div className="iii-cloud-cell">
        <span className="iii-cloud-cell-label">Health</span>
        {!serving ? (
          <span className="iii-cloud-cell-value">Nothing runs on {env.name} yet</span>
        ) : health.error ? (
          <span className="iii-cloud-cell-value">iii cloud did not answer</span>
        ) : h && health.data ? (
          <>
            <span className="iii-cloud-cell-value">
              {h.tone === 'ok' ? null : <Dot tone={h.tone} />} {h.text}
            </span>
            <span className="iii-cloud-desc">checked {clock(health.data.checked_at)}</span>
          </>
        ) : (
          <Skeleton className="iii-cloud-skeleton-row" />
        )}
      </div>
      <div className="iii-cloud-cell">
        <span className="iii-cloud-cell-label">Deploys from</span>
        <span className="iii-cloud-cell-value">{words.value}</span>
        <span className="iii-cloud-desc">{words.detail}</span>
      </div>
    </Panel>
  )
}

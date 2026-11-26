import type { Host } from '@iii-dev/console-ui'
import { Button, Panel, Skeleton } from '@iii-dev/console-ui'
import { useCloud } from './api'
import { clock } from './asked'
import { failedLine } from './FailureCard'
import { needsYou } from './findings'
import { useActions, useLocked } from './hub'
import {
  defaultBranch,
  deployOrigin,
  deployRows,
  fixBranch,
  isDirectEnv,
  isFailedDeploy,
  minutesLabel,
  needsAction,
  plural,
  roleOf,
  servingDeploy,
  sha7,
  sourceLabel,
  type Tone,
  when,
  withAppFile,
} from './model'
import { Dot, ErrorPanel, Mono, SectionCard as Section, StatePill } from './parts'
import type { EnvTabProps, Route } from './route'
import type { CloudDeployment, DeploymentList, Members, Status, WorkerError } from './types'

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

/**
 * The one thing to do next in the selected environment `envId`, the first that holds, in this order:
 *  1. held: a deployment iii cloud holds for the person's approval
 *  2. running: a deployment is going
 *  3. you: the latest check has problems only the person can solve
 *  4. agent: the agent is working, or the check is running, and nothing needs the person
 *  5. passed: a check passed on a commit the environment does not serve (not one whose deployment just failed)
 *  6. failed: the newest deployment failed (unless the folder has moved past that commit: then 9)
 *  7. pull-request: nothing is live, the environment deploys from a pull request, and one is open
 *  8. deploy: the folder has commits or uncommitted files the environment does not serve
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
  const you = c && !c.ready && c.run?.status !== 'running' ? needsYou(s) : null
  const n = (you?.items.length ?? 0) + (you?.decide.length ?? 0)
  const a = s.agent && ['starting', 'working'].includes(s.agent.state) ? s.agent : null
  const fixing = a?.working_on?.length ?? 0
  if (you && n) {
    const first = you.items[0]?.title ?? `${name}'s ${you.decide[0].setting} was changed in iii cloud's console`
    return {
      state: 'you',
      wait: true,
      primary: true,
      title: `${plural(n, 'thing')} ${n === 1 ? 'needs' : 'need'} you before ${name} can get ${branch ?? 'your changes'}`,
      detail: `${first}${n > 1 ? ` and ${n - 1} more` : ''}.${a && fixing ? ` Your agent is fixing ${plural(fixing, 'problem')} meanwhile.` : ''}`,
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
  const folderMoved =
    files > 0 || (notServed.length > 0 && !(failed && sameCommit(notServed[0].sha, failed.commit.sha)))
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

  if (!live && env && !isDirectEnv(s, env) && s.pr?.state === 'OPEN') {
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

  if (files > 0 || notServed.length > 0) {
    const top = notServed[0]
    const rest = files ? `plus ${plural(files, 'uncommitted file')}` : 'nothing uncommitted in your folder'
    return {
      state: 'deploy',
      wait: true,
      primary: true,
      title: notServed.length
        ? `${plural(notServed.length, 'commit')} on ${branch ?? 'your branch'} ${notServed.length === 1 ? "isn't" : "aren't"} on ${name}`
        : `${plural(files, 'uncommitted file')} in your folder ${files === 1 ? "isn't" : "aren't"} on ${name}`,
      detail: top ? `${sha7(top.sha)} ${top.message.split('\n')[0]} · ${rest}` : 'Commit them, or deploy as they are.',
      action: { label: `Deploy to ${name}`, kind: 'check' },
    }
  }

  if (!live) {
    return {
      state: 'empty',
      wait: false,
      primary: false,
      title: `Nothing is live on ${name} yet`,
      detail:
        env && !isDirectEnv(s, env)
          ? 'It deploys when a pull request into main is merged.'
          : 'Deploy from this machine to start it.',
      action: { label: 'Open the check', kind: 'go', to: { at: 'deploy', env: envId } },
    }
  }
  return {
    state: 'current',
    wait: false,
    primary: false,
    title: `${name} serves everything in your folder`,
    detail: `${branch ?? 'Your branch'}${p ? ` at ${sha7(p.checkout.head)}` : ''}, nothing uncommitted.`,
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
        <span className="iii-cloud-desc">Next</span>
        <h3 className="iii-cloud-next-title">{next.title}</h3>
        <p className="iii-cloud-desc">{next.detail}</p>
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
}: Pick<EnvTabProps, 'host' | 'status' | 'envId'> & {
  feed: { data: DeploymentList | null; error: WorkerError | null }
}) {
  const p = status.project
  const env = p?.environments.find((e) => e.id === envId)
  const serving = servingOf(status, envId, feed.data?.deployments)
  const health = useCloud<Health>(host, status, envId, serving ? '/health' : null)
  if (!env) return null
  const h = health.data ? healthOf(health.data, env.name) : null
  const direct = isDirectEnv(status, env)
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
              <Dot tone={h.tone} /> {h.text}
            </span>
            <span className="iii-cloud-desc">checked {clock(health.data.checked_at)}</span>
          </>
        ) : (
          <Skeleton className="iii-cloud-skeleton-row" />
        )}
      </div>
      <div className="iii-cloud-cell">
        <span className="iii-cloud-cell-label">Deploys from</span>
        <span className="iii-cloud-cell-value">
          {direct ? 'this machine, after the check' : 'when its pull request is merged'}
        </span>
        <span className="iii-cloud-desc">
          {direct
            ? `a push to ${env.name}'s branch deploys without it`
            : `into ${status.pr?.base ?? defaultBranch(status) ?? 'its base branch'}`}
        </span>
      </div>
    </Panel>
  )
}

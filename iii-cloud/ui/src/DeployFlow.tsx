// The Deploy screen of an environment, one screen driven by state: the check and what this deploy does (CheckStage),
// the deployment itself (stages, then the address or what went wrong), and the Change panel under the header.
// Only the person deploys: every push to an environment's branch, merge and deploy here is one of their clicks.

import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  ConfirmDialog,
  EmptyState,
  List,
  ListItem,
  PageMain,
  Panel,
  Skeleton,
  TerminalStream,
} from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, Circle, CircleDot, Copy } from 'lucide-react'
import { type ReactNode, type RefObject, useEffect, useRef, useState } from 'react'
import { cloudWrite, useCloud } from './api'
import { clock, useLaunch } from './asked'
import {
  ActionBar,
  ActionError,
  CheckStage,
  checkPayload,
  deployLockReason,
  GoesSub,
  heldElsewhere,
  retryPayload,
  ScreenHead,
  TypedConfirm,
} from './CheckStage'
import { useDeployments } from './EnvOverview'
import { handOff, openAgentChat } from './handoff'
import { hubOf, useActions, useCheckError, useLocked } from './hub'
import { Deleted } from './Manage'
import {
  activeSameKind,
  ago,
  appFile,
  COMPOSE_ISSUE,
  cloudStageLabel,
  composeFile,
  deployOrigin,
  deployState,
  deployWord,
  envPageUrl,
  failureWords,
  isFailedDeploy,
  listMayHideServing,
  minutesLabel,
  needsAction,
  planCodeWords,
  plural,
  RESOURCE_KIND,
  type StageRow,
  sendLabel,
  servingId,
  servingNamed,
  sha7,
  stageKeyOf,
  stageRows,
} from './model'
import {
  Dot,
  Fresh,
  LinkButton,
  Mono,
  Notice,
  PageAction,
  PathBar,
  SectionCard,
  StatePill,
  secretsNote,
  uncommittedCount,
  useNow,
} from './parts'
import { type AreaProps, envState, pathOf, type RouteAt } from './route'
import type { BranchRef, Deploy, Status } from './types'

type Act = ReturnType<typeof useActions>
type Health = { status: string; workers: { ready: boolean }[] }
type LogPage = { lines: { worker: string | null; message: string }[] }

const span = (from: string, to: string) => Date.parse(to) - Date.parse(from)
/** "stopped while starting the workers": the cloud's stage names, as an activity. */
const WHILE: Record<string, string> = {
  validation: 'validating the plan',
  build: 'building the image',
  resources: 'creating resources',
  config_secrets: 'applying config and secrets',
  release: 'running the release command',
  workers: 'starting the workers',
  routes_tls: 'setting up the address',
}

const DONE: Record<string, string> = {
  build: 'Built the image',
  data: 'Database, secrets and settings',
  workers: 'Started the workers',
  routes: 'Address and certificate',
}
const DOING: Record<string, string> = { ...DONE, build: 'Building the image', workers: 'Starting the workers' }
/** A stage row's words, as the canvas has them: done in the past tense, the one going or stopped in the present. */
function stageSays(r: StageRow, d: Deploy, env: string, machine: boolean): ReactNode {
  const sha = sha7(d.commit.sha)
  if (r.key === 'plan') {
    // "Pushed" only when this page started it; a redeploy or a rollback made in the console pushed nothing.
    return machine && r.tone === 'ok' ? (
      <>
        Pushed <Mono>{sha}</Mono> {`to ${env}'s branch and started #${d.number}`}
      </>
    ) : (
      <>
        {`Deployment #${d.number} of `}
        <Mono>{sha}</Mono> started
      </>
    )
  }
  return (r.tone === 'ok' ? DONE : DOING)[r.key] ?? r.short
}

/** A choice: its action, and one line under it saying what it does. */
function Choice({ line, children }: { line: string; children: ReactNode }) {
  return (
    <div className="iii-cloud-cs-choice">
      {children}
      <span className="iii-cloud-cs-sub">{line}</span>
    </div>
  )
}

/** Deploying from this page: one header, then the body the state asks for. */
export function DeployArea({ host, status, route, go, narrow }: AreaProps<RouteAt<'deploy'>>) {
  const act = useActions(host)
  const env = route.env
  const state = envState(status, env)
  const [changing, setChanging] = useState(false)
  const trigger = useRef<HTMLElement | null>(null)
  const checkAgain = () => void act.call('check', 'iii-cloud::check', retryPayload(host, status))

  // The deployment the route names, if it is this environment's; a failed one the agent is already fixing goes
  // back to the check, where the agent is.
  const asked = status.deploys.find((d) => d.id === route.deploy)
  const shown =
    asked && asked.environment_id === env && !(isFailedDeploy(asked) && activeSameKind(status, 'deploy_fix'))
  const name = status.project?.environments.find((e) => e.id === env)?.name ?? 'the environment'

  // What the last check call of this environment was refused with: kept by the hub, so it shows after the card that
  // asked is gone (the environment page and first-time setup go here at once).
  const refused = useCheckError(host, env)
  // A check that arrives replaces whatever the last call here was refused with (a refusal is about the old state).
  const { setError } = act
  const checkId = status.check?.id
  useEffect(() => {
    if (checkId) setError(null)
  }, [checkId, setError])
  const gone =
    status.problems?.find((p) => p.code === 'PROJECT_DELETED') ??
    (state === 'selected' ? status.problems?.find((p) => p.code === 'ENVIRONMENT_DELETED') : undefined)

  let body: ReactNode
  if (gone) {
    body = <Deleted host={host} status={status} problem={gone} />
  } else if (state === 'unknown') {
    body = (
      <EmptyState
        title="Not an environment of this project"
        description="It may have been deleted. Pick one from the project's environments."
        action={{ label: 'Back to Projects', onClick: () => go({ at: 'start' }) }}
      />
    )
  } else if (state === 'switching') {
    body = <Skeleton className="iii-cloud-skeleton" />
  } else if (route.deploy && !asked) {
    body = (
      <EmptyState
        title="Deployment not found"
        description={`It isn't a deployment of ${name} this page knows. Its environment has the list.`}
        action={{ label: `Back to ${name}`, onClick: () => go({ at: 'env', env }) }}
      />
    )
  } else if (shown && asked) {
    body = <Follow host={host} status={status} d={asked} act={act} checkAgain={checkAgain} go={go} narrow={narrow} />
  } else {
    body = (
      <CheckStage
        host={host}
        status={status}
        go={go}
        narrow={narrow}
        onChange={(button) => {
          trigger.current = button
          setChanging(!changing)
        }}
        panel={
          changing ? (
            <ChangePanel
              host={host}
              status={status}
              envId={env}
              act={act}
              go={go}
              onClose={() => setChanging(false)}
              trigger={trigger}
            />
          ) : null
        }
      />
    )
  }

  return (
    <PageMain className="iii-cloud-main">
      <PathBar
        path={pathOf(route, status, narrow)}
        host={host}
        status={status}
        route={route}
        go={go}
        narrow={narrow}
        rule
        end={<Fresh host={host} status={status} narrow={narrow} />}
      />
      <div className="iii-cloud-body iii-cloud-scroll">
        <div className="iii-cloud-depcol">
          {act.error ? (
            <ActionError host={host} status={status} error={act.error} checkAgain={checkAgain} go={go} />
          ) : refused && !gone ? (
            <ActionError host={host} status={status} error={refused} checkAgain={checkAgain} go={go} />
          ) : null}
          {body}
        </div>
      </div>
    </PageMain>
  )
}

/* ── Change: what goes ─────────────────────────────────────────────── */

type Source = {
  value: string
  label: string
  /** The branch `use` takes; none: the folder's own HEAD. */
  ref: BranchRef | null
  folder: boolean
  /** "3 commits dev doesn't have"; none when the environment's branch isn't on origin. */
  ahead?: string
  default?: boolean
}

/**
 * The branches a check can start from (`checkout.branches`; none on an older worker): the folder's own first,
 * then the one the environment deploys, then the others, newest first. The folder's is the default when it
 * has something the environment lacks, else the environment's own.
 */
function sourcesOf(p: NonNullable<Status['project']>, env: string): Source[] | null {
  const refs = p.checkout.branches
  if (!refs?.length) return null
  const lacks = (n: number | null | undefined) =>
    n == null ? undefined : n ? `${plural(n, 'commit')} ${env} doesn't have` : `nothing ${env} doesn't have`
  const here = p.checkout.branch
  const mine = refs.find((r) => !r.remote && r.name === here)
  const envRef = refs.find((r) => r.is_env_branch && r !== mine)
  const folderAhead = mine ? mine.ahead : (p.checkout.not_served?.length ?? null)
  const out: Source[] = [
    { value: '', label: here ?? sha7(p.checkout.head), ref: null, folder: true, ahead: lacks(folderAhead) },
    ...[envRef, ...refs.filter((r) => r !== mine && r !== envRef)]
      .filter((r): r is BranchRef => !!r)
      .map((r) => ({ value: r.name, label: r.name, ref: r, folder: false, ahead: lacks(r.ahead) })),
  ]
  const pick = (folderAhead ?? 0) > 0 || !envRef ? out[0] : out[1]
  pick.default = true
  return out
}

/** The branch, the uncommitted files and the compose file the check runs on. Confirming stops the check in flight and starts another. */
function ChangePanel({
  host,
  status,
  envId,
  act,
  go,
  onClose,
  trigger,
}: {
  host: Host
  status: Status
  envId: string
  act: Act
  go: AreaProps['go']
  onClose: () => void
  /** The "Change" button that opened it: the focus goes back to that one (a screen can have two panes). */
  trigger: RefObject<HTMLElement | null>
}) {
  const [pick, setPick] = useState(() => appFile(status).path)
  const [source, setSource] = useState<string | null>(null)
  const top = useRef<HTMLDivElement>(null)
  // The panel takes focus when it opens and gives it back to "Change" when it closes.
  useEffect(() => {
    top.current?.focus()
    const back = trigger.current
    return () => back?.focus()
  }, [trigger])
  // Named by iii cloud's list; until it is read, no deployment is named.
  const feed = useDeployments(host, status, envId)
  const p = status.project
  const env = p?.environment
  if (!p || !env) return null
  const files = status.compose_files ?? []
  const current = composeFile(env.manifest)
  const changed = uncommittedCount(p.checkout.changed_files)
  const secrets = secretsNote(p.checkout.changed_files)
  const sources = env.production ? null : sourcesOf(p, env.name)
  const chosen = sources ? (sources.find((o) => o.value === source) ?? sources.find((o) => o.default)) : undefined
  const serving = servingNamed(status, envId, feed.data?.deployments)

  const start = async () => {
    // The agent works in the copy this replaces: it stops first, and its commits stay where they are. A step that
    // fails ends it there, with its error on screen: nothing replaces a copy the agent still works in.
    if (status.agent && ['starting', 'working', 'waiting_for_you', 'waiting', 'stalled'].includes(status.agent.state)) {
      const stopped = await act.call('stop', 'iii-cloud::handoff::stop', { id: status.agent.handoff_id })
      if (stopped === null) return
    }
    const used = await act.call(
      'use',
      'iii-cloud::project::use',
      {
        path: p.path,
        environment_id: envId,
        mode: p.workspace?.mode ?? 'worktree',
        // Production goes through a pull request: its check starts from the environment, never the checkout.
        from: env.production ? 'environment' : 'checkout',
        ...(chosen?.ref ? { source: chosen.ref.name } : {}),
      },
      { changesSource: true },
    )
    if (used === null) return
    // The old check stays stale until another one exists, and a retry after a refusal checks the file picked here.
    hubOf(host).setPickedManifest(envId, pick)
    onClose()
    void act.call('check', 'iii-cloud::check', checkPayload(status, pick))
    go({ at: 'deploy', env: envId })
  }

  const one = changed === 1
  return (
    <>
      <SectionCard
        title={`What goes to ${env.name}`}
        note={
          serving
            ? `${env.name} serves ${sha7(serving.sha)} · deployment #${serving.number}`
            : feed.data
              ? listMayHideServing(feed.data)
                ? `${env.name} serves a deployment iii cloud's list does not show`
                : `Nothing is live on ${env.name} yet`
              : null
        }
        flush
      >
        <div className="iii-cloud-cs-change" ref={top} tabIndex={-1}>
          <div className="iii-cloud-cs-crow">
            <span className="iii-cloud-cs-clabel" aria-hidden>
              Code
            </span>
            <fieldset className="iii-cloud-cs-fieldset">
              <legend className="iii-cloud-cs-sr">Code</legend>
              {sources && chosen ? (
                <List role="radiogroup" aria-label="Code" className="iii-cloud-cs-choices">
                  {sources.map((o) => (
                    <ListItem
                      key={o.value}
                      role="radio"
                      aria-checked={o === chosen}
                      // `data-selected`, not `selected`: a radio has no pressed state.
                      data-selected={o === chosen ? '' : undefined}
                      leading={o === chosen ? <CircleDot /> : <Circle />}
                      label={o.label}
                      trailing={[o.folder ? 'your folder' : null, o.ahead].filter(Boolean).join(' · ')}
                      onClick={() => setSource(o.value)}
                    />
                  ))}
                </List>
              ) : (
                <p className="iii-cloud-cs-sub">
                  {env.production
                    ? `${env.name}'s check starts from ${env.branch}. Your changes reach it through a pull request.`
                    : `The check starts from ${p.checkout.branch ?? sha7(p.checkout.head)}.`}
                </p>
              )}
              {!env.production && (chosen?.folder ?? true) && (changed || secrets) ? (
                <p className="iii-cloud-cs-sub">
                  {[
                    changed
                      ? `The ${plural(changed, 'uncommitted change')} in your folder ${one ? 'stays' : 'stay'} there: commit ${one ? 'it to include it' : 'them to include them'}.`
                      : null,
                    secrets,
                  ]
                    .filter(Boolean)
                    .join(' ')}
                </p>
              ) : null}
            </fieldset>
          </div>
          <div className="iii-cloud-cs-crow">
            <span className="iii-cloud-cs-clabel" aria-hidden>
              Compose file
            </span>
            <fieldset className="iii-cloud-cs-fieldset">
              <legend className="iii-cloud-cs-sr">Compose file</legend>
              <List role="radiogroup" aria-label="Compose files in this folder" className="iii-cloud-cs-choices">
                {(files.length ? files : [{ path: current, choosable: true, workers: null, issue: null }]).map((f) => {
                  const on = f.path === pick
                  const note = [
                    f.workers != null ? plural(f.workers, 'worker') : null,
                    f.path === current ? `what ${env.name} runs now` : `${env.name} switches to it with this deploy`,
                    f.choosable ? null : (COMPOSE_ISSUE[f.issue ?? ''] ?? f.issue),
                  ]
                    .filter(Boolean)
                    .join(' · ')
                  return (
                    <ListItem
                      key={f.path}
                      role="radio"
                      aria-checked={on}
                      data-selected={on ? '' : undefined}
                      disabled={!f.choosable}
                      leading={on ? <CircleDot /> : <Circle />}
                      label={<Mono>{f.path}</Mono>}
                      trailing={note}
                      onClick={() => setPick(f.path)}
                    />
                  )
                })}
              </List>
            </fieldset>
          </div>
          <div className="iii-cloud-cs-crow">
            <span className="iii-cloud-cs-clabel">Environment</span>
            <p className="iii-cloud-cs-sub">
              {env.name} · to deploy somewhere else, open that environment from the sidebar or{' '}
              <button type="button" className="iii-cloud-linkbtn" onClick={() => go({ at: 'start' })}>
                Projects
              </button>
            </p>
          </div>
          <div className="iii-cloud-cs-changefoot">
            <Button variant="primary" disabled={act.busy === 'use' || !env} onClick={() => void start()}>
              Check this instead
            </Button>
            <button type="button" className="iii-cloud-linkbtn" onClick={onClose}>
              Cancel
            </button>
            <span className="iii-cloud-cs-sub">
              A change stops the running check and your agent, and starts a new check.
            </span>
          </div>
        </div>
      </SectionCard>
      <p className="iii-cloud-cs-foot">
        Until you confirm a change, the check that is running goes on. The lists below this panel stay where they are.
      </p>
    </>
  )
}

/* ── The deployment itself ─────────────────────────────────────────── */

function Follow({
  host,
  status,
  d,
  act,
  checkAgain,
  go,
  narrow,
}: {
  host: Host
  status: Status
  d: Deploy
  act: Act
  checkAgain: () => void
  go: AreaProps['go']
  narrow: boolean
}) {
  const now = useNow(15_000)
  const launching = useLaunch()?.kind === 'launching'
  const [confirming, setConfirming] = useState(false)
  const [canceling, setCanceling] = useState(false)
  const stale = useLocked(host, status)
  const envId = d.environment_id
  const running = d.outcome === 'running' && !needsAction(d)
  // Live is what iii cloud's list says serves: an old Follow card of a replaced deployment is not.
  const feed = useDeployments(host, status, envId)
  const live = d.outcome === 'ready' && d.id === servingId(status, envId, feed.data?.deployments)
  const held = needsAction(d)
  const failed = isFailedDeploy(d)
  // The worker reads a deploy's log only once it ends: while it runs, the page reads the cloud's.
  const logs = useCloud<LogPage>(host, status, envId, running ? `/logs?deployment_id=${d.id}` : null, { pollMs: 5000 })
  const health = useCloud<Health>(host, status, envId, live ? '/health' : null)
  const env = status.project?.environment
  const address = env?.url ?? null
  const copy = useCopyFlash(address ?? '')
  if (!env) return null
  const sha = sha7(d.commit.sha)
  const prev = status.deploys.find(
    (x) =>
      x.environment_id === envId &&
      x.id !== d.id &&
      x.outcome === 'ready' &&
      Date.parse(x.created_at) < Date.parse(d.created_at),
  )
  const c = status.check
  // A held deployment is this page's only when the worker's own markers say so (a check_id, an approval, or the record
  // it created). One without them is "elsewhere": a console push the worker never saw, which a check cannot confirm.
  const elsewhere = heldElsewhere(d)
  // A deployment this page started names the check it came from, but every new check has a new id: a passed check of
  // the same commit lets it be confirmed (the worker's `deployment_id` path accepts the same commit, and pins the
  // plan to the new check's digest).
  const ours = !!c && !elsewhere && (d.check_id === c.id || d.commit.sha === c.commit)
  // It carries this page's own marks: it was started here, and no passed check matches it any more.
  const startedHere = !!d.check_id || d.approved_by_worker
  const confirmable = !!c?.ready && ours
  // Held, and its plan has blocking problems (an error of the validation stage): it can never be confirmed.
  const blocked = held && d.error?.stage === 'validation'
  const stage = d.failure?.stage ?? d.error?.stage ?? null
  const stageWords = stage ? `while ${WHILE[stage] ?? cloudStageLabel(stage).toLowerCase()}` : null
  const title = blocked
    ? `Deployment #${d.number} can't start as it is`
    : elsewhere
      ? `Deployment #${d.number} is waiting for confirmation in iii cloud`
      : held
        ? confirmable
          ? `Deployment #${d.number} needs your action`
          : `Deployment #${d.number} waits for your approval`
        : running
          ? `Deploying ${sha} to ${env.name}`
          : live
            ? `${env.name} is serving ${sha}`
            : failed
              ? `Deployment #${d.number} stopped${stageWords ? ` ${stageWords}` : ''}`
              : `Deployment #${d.number}: ${deployWord(d)}`
  const sub = blocked
    ? `Deployment #${d.number} · iii cloud holds it, and the plan's problems keep it from starting`
    : elsewhere
      ? `Deployment #${d.number} · not started from this page; deploying from the check replaces it`
      : held
        ? `Deployment #${d.number} · iii cloud holds it until you confirm`
        : running
          ? `Deployment #${d.number} · started ${clock(d.created_at)} · ${minutesLabel(now - Date.parse(d.created_at))} so far`
          : live && d.finished_at
            ? `Deployment #${d.number} · live at ${clock(d.finished_at)} · took ${minutesLabel(span(d.created_at, d.finished_at))}`
            : failed
              ? prev
                ? `${env.name} still serves #${prev.number}. Nothing changed for your users.`
                : `Nothing was live on ${env.name} before it.`
              : `Deployment #${d.number} · ${ago(d.finished_at ?? d.created_at)}`
  const checkedSha = status.checks?.find((x) => x.id === d.check_id)?.commit ?? (ours ? c?.commit : null)
  const workers = health.data?.workers ?? []
  const { resources, overrides } = (ours ? c?.plan.confirmations : null) ?? { resources: [], overrides: [] }
  const approves = [
    ...resources.map((r) => `a shared ${RESOURCE_KIND[r.kind] ?? r.kind} for ${r.binding ?? r.alias}`),
    overrides.length ? `${plural(overrides.length, 'setting')} from the compose file` : null,
  ].filter(Boolean)
  const deployLock = deployLockReason(status, envId, { stale, sending: act.busy === 'deploy', exceptId: d.id })
  const confirmPayload = { check_id: c?.id, deployment_id: d.id }
  const confirm = async () => {
    const out = await act.call<Deploy>('deploy', 'iii-cloud::deploy', confirmPayload)
    if (out) await hubOf(host).refresh(false)
  }
  // Newest first from the cloud; the last lines, oldest first.
  const logText = (logs.data?.lines ?? [])
    .slice(0, 12)
    .reverse()
    .map((l) => (l.worker ? `${l.worker}: ${l.message}` : l.message))
    .join('\n')
  // The dialog stays open and busy until this settles, then closes itself; a refusal shows in it.
  const cancel = async () => {
    await cloudWrite(host, status, envId, 'POST', `/deployments/${d.id}/cancel`)
    void hubOf(host).refresh(true)
  }
  const heldLinks = (
    <span className="iii-cloud-row">
      <button type="button" className="iii-cloud-linkbtn iii-cloud-small" onClick={() => setCanceling(true)}>
        {`Cancel deployment #${d.number}`}
      </button>
      {d.console_url ? (
        <a className="iii-cloud-link iii-cloud-small" href={d.console_url} target="_blank" rel="noreferrer">
          {blocked || confirmable ? 'Open it in iii cloud' : 'Approve it in iii cloud'}
        </a>
      ) : null}
    </span>
  )
  // What stays live is iii cloud's list (a rollback in the console moves it), never the worker's memory.
  const serving = servingNamed(status, envId, feed.data?.deployments)
  const servingSays = serving
    ? `${env.name} keeps serving #${serving.number}.`
    : feed.data
      ? listMayHideServing(feed.data)
        ? `${env.name} keeps serving a deployment iii cloud's list does not show.`
        : `Nothing is live on ${env.name}, so nothing changes for your users.`
      : `iii cloud's list of ${env.name} is not read yet.`
  const cloudLog = d.console_url || envPageUrl(status, 'deployments', env.name)
  const chatOpen = activeSameKind(status, 'deploy_fix')

  let action: ReactNode = null
  if (live) action = <PageAction onClick={() => go({ at: 'env', env: envId })}>{`Back to ${env.name}`}</PageAction>
  else if (failed)
    action = chatOpen ? (
      <PageAction onClick={() => openAgentChat(host, status)}>Open chat</PageAction>
    ) : (
      <PageAction
        disabled={launching || !host.chat?.compose}
        onClick={async () => {
          // The fix goes through the check again, on this screen.
          if (await handOff(host, status, 'deploy_fix', d)) go({ at: 'deploy', env: envId })
        }}
      >
        {sendLabel(status, 'deploy_fix', 'Fix with your agent')}
      </PageAction>
    )
  const note = failed
    ? 'Your agent works on your computer, in a separate copy. Its fix goes through the check, and you confirm the deploy.'
    : null

  return (
    <div className="iii-cloud-cs" data-narrow={narrow ? '' : undefined}>
      <ScreenHead
        title={title}
        chip={<StatePill state={deployState(d, servingId(status, envId, feed.data?.deployments))} />}
        sub={
          <>
            <p className="iii-cloud-cs-sub">{sub}</p>
            {ours && !live && !failed ? <GoesSub s={status} c={c} env={env.name} /> : null}
          </>
        }
        action={action}
        narrow={narrow}
      />
      {running && status.connection.status === 'expired' ? (
        <Notice
          variant="warn"
          headline="Signed out while it deploys"
          detail="The deployment goes on in iii cloud. Sign in again to follow it here."
          action={
            <Button
              size="sm"
              variant="primary"
              onClick={() => act.call('connect', 'iii-cloud::connect', { method: 'browser' })}
            >
              Sign in again
            </Button>
          }
        />
      ) : null}
      {blocked ? (
        <Notice
          className="iii-cloud-cs-stop"
          variant="warn"
          headline={`iii cloud's plan for ${sha} has blocking problems`}
          detail={`${
            planCodeWords(status, d.error?.hint ?? null).length
              ? `${planCodeWords(status, d.error?.hint ?? null).join('; ')}.`
              : (d.error?.hint ?? '')
          } A deployment like this is never started: it waits until you cancel it or a fixed commit replaces it.`}
          action={
            <div className="iii-cloud-cs-decide">
              <Choice
                line={`Your agent checks ${sha} for ${env.name} in a clean copy, fixes what is its to fix and tells you what is yours.`}
              >
                <Button
                  size="sm"
                  variant="primary"
                  disabled={launching || !host.chat?.compose}
                  onClick={async () => {
                    if (await handOff(host, status, 'deploy_fix', d)) go({ at: 'deploy', env: envId })
                  }}
                >
                  {sendLabel(status, 'deploy_fix', 'Fix with your agent')}
                </Button>
              </Choice>
              <Choice line="Lists every problem with its file and line. Nothing is deployed.">
                <Button size="sm" variant="secondary" onClick={checkAgain}>
                  {`Check ${env.name} here`}
                </Button>
              </Choice>
              {heldLinks}
            </div>
          }
        />
      ) : held && elsewhere ? (
        <Notice
          variant="info"
          headline={`Deployment #${d.number} was not started from this page`}
          detail={`iii cloud holds it for confirmation, and nothing here can confirm it. Deploying from the check replaces it.`}
          action={
            <Button size="sm" variant="primary" onClick={() => go({ at: 'deploy', env: envId })}>
              Go to the check
            </Button>
          }
        />
      ) : held && confirmable ? (
        <Notice
          className="iii-cloud-cs-stop"
          variant="warn"
          headline="Needs your action"
          detail={`iii cloud holds this deployment until you approve what it creates and changes${approves.length ? `: ${approves.join(', ')}` : ''}.`}
          action={
            <div className="iii-cloud-cs-decide">
              <Choice line="Approves what it creates and changes, and lets it go on.">
                <Button
                  size="sm"
                  variant="primary"
                  disabled={!!deployLock}
                  title={deployLock ?? undefined}
                  onClick={() => (env.production ? setConfirming(true) : void confirm())}
                >
                  Confirm deploy
                </Button>
              </Choice>
              {heldLinks}
            </div>
          }
        />
      ) : held ? (
        <Notice
          className="iii-cloud-cs-stop"
          variant="warn"
          headline={startedHere ? 'No passed check matches it' : 'iii cloud holds it for your approval'}
          detail={
            startedHere
              ? `No passed check of ${sha} matches deployment #${d.number} any more. Check ${env.name} here: if that check passes on ${sha}, you confirm here; if it checks another commit, approve or cancel it in iii cloud.`
              : `It was not started from this page, so nothing here has checked ${sha}. Check ${env.name} first: when a check of ${sha} passes, you confirm here.`
          }
          action={
            <div className="iii-cloud-cs-decide">
              <Choice line="Runs iii cloud's plan and the local run on this environment. Nothing is deployed.">
                <Button size="sm" variant="primary" onClick={checkAgain}>
                  {`Check ${env.name} here`}
                </Button>
              </Choice>
              {heldLinks}
            </div>
          }
        />
      ) : null}

      {failed ? (
        <Panel className="iii-cloud-panel" role="region" aria-label="What happened">
          <div className="iii-cloud-cs-cause">
            <span className="iii-cloud-cs-sub">
              {`What happened${d.checked_ready ? ` · the check passed on ${checkedSha ? sha7(checkedSha) : 'this commit'}, and only the cloud could see this` : ''}`}
            </span>
            <h3>
              {d.failure?.title ??
                `The deployment stopped${stageWords ? ` ${stageWords}` : ''}${d.error ? `: ${failureWords(d.error)}` : ''}`}
            </h3>
            {(d.failure?.hint ?? d.error?.hint) ? <p>{d.failure?.hint ?? d.error?.hint}</p> : null}
            <TerminalStream label="log" text={(d.failure?.log_tail ?? d.log_tail).join('\n')} />
          </div>
          <div className="iii-cloud-cs-causefoot">
            <span className="iii-cloud-cs-sub">
              Your agent gets these log lines. Its fix goes through the check again, on this screen.
            </span>
            {cloudLog ? (
              <a className="iii-cloud-link iii-cloud-small" href={cloudLog} target="_blank" rel="noreferrer">
                Full log in iii cloud
              </a>
            ) : null}
          </div>
        </Panel>
      ) : null}

      <Panel className="iii-cloud-panel" role="region" aria-label="Deployment stages">
        <ol className="iii-cloud-cs-stages">
          {stageRows(d).map((r) => (
            <li key={r.key} className="iii-cloud-cs-stage" data-tone={r.tone}>
              <span className="iii-cloud-stagedot" aria-hidden />
              <span className="iii-cloud-cs-slabel">
                {stageSays(r, d, env.name, deployOrigin(status, d).from === 'machine')}
              </span>
              {held && r.tone === 'alert' && d.error && stageKeyOf(d.error.stage) === r.key ? (
                <span className="iii-cloud-cs-sreason">{d.error.hint || failureWords(d.error)}</span>
              ) : null}
              <span className="iii-cloud-cs-state">
                {r.key === 'plan' && r.tone === 'ok' ? clock(d.created_at) : r.time}
              </span>
            </li>
          ))}
        </ol>
      </Panel>

      {live ? (
        <Panel className="iii-cloud-panel" role="region" aria-label="After the deploy">
          {address ? (
            <div className="iii-cloud-cs-url">
              <span className="iii-cloud-cs-addr">{address}</span>
              <span className="iii-cloud-row">
                <LinkButton href={address}>Open</LinkButton>
                <Button size="sm" variant="secondary" onClick={copy.copy}>
                  {copy.state === 'copied' ? (
                    <>
                      <Check />
                      Copied
                    </>
                  ) : (
                    <>
                      <Copy />
                      Copy
                    </>
                  )}
                </Button>
              </span>
            </div>
          ) : null}
          <ul className="iii-cloud-cs-facts" aria-label="Health">
            {health.data ? (
              <li>
                {health.data.status === 'healthy' ? (
                  <Check className="iii-cloud-mark" data-mark="ok" aria-hidden />
                ) : (
                  <Dot tone="warn" />
                )}
                <span>{`iii cloud says it's ${health.data.status}`}</span>
                <span className="iii-cloud-cs-state">
                  {workers.filter((w) => w.ready).length} of {plural(workers.length, 'worker')} up
                </span>
              </li>
            ) : health.error ? (
              <li>
                <Dot tone="ghost" />
                <span>{health.error.message}</span>
              </li>
            ) : (
              <li>
                <Skeleton className="iii-cloud-cs-skel" />
              </li>
            )}
          </ul>
        </Panel>
      ) : null}

      {running ? (
        <>
          {logText ? <TerminalStream label="Live log" text={logText} /> : null}
          <p className="iii-cloud-cs-foot">
            {prev ? `${env.name} keeps serving #${prev.number} until #${d.number} is live. ` : ''}You can leave this
            page; the deployment goes on, and the console tells you when it's done. If it fails, Fix with your agent
            gets its log lines.
          </p>
          {deployOrigin(status, d).from === 'machine' ? (
            <div className="iii-cloud-row">
              <Button variant="secondary" size="sm" onClick={() => setCanceling(true)}>
                Cancel the deployment
              </Button>
            </div>
          ) : null}
        </>
      ) : null}
      {running || held ? (
        <ConfirmDialog
          open={canceling}
          onOpenChange={setCanceling}
          title={`Cancel deployment #${d.number}?`}
          description={
            held
              ? `iii cloud drops it. ${servingSays} Nothing is deleted.`
              : 'iii cloud stops it at the stage it is in. Nothing is deleted.'
          }
          confirmLabel={`Cancel deployment #${d.number}`}
          cancelLabel={held ? 'Keep it' : 'Keep deploying'}
          tone="danger"
          onConfirm={cancel}
        />
      ) : null}
      {live ? (
        <p className="iii-cloud-cs-foot">
          {`While a deploy runs, this screen shows the same stages with the running one marked, and ${env.name} keeps serving the previous deployment. You can leave; the console tells you when it's done.`}
          {prev && envPageUrl(status, `deployments/${prev.number}`, env.name) ? (
            <>
              {" If something's off, "}
              <a
                href={envPageUrl(status, `deployments/${prev.number}`, env.name) as string}
                target="_blank"
                rel="noreferrer"
              >
                roll back to #{prev.number} in iii cloud
              </a>
              .
            </>
          ) : null}
        </p>
      ) : null}
      <ActionBar narrow={narrow} action={action} note={note} />
      <TypedConfirm
        open={confirming}
        onOpenChange={setConfirming}
        name={env.name}
        title={`Confirm deployment #${d.number} to ${env.name}?`}
        description={`This approves what deployment #${d.number} creates and changes, so iii cloud lets it go on.`}
        confirmLabel="Confirm deploy"
        onConfirm={confirm}
        locked={deployLockReason(status, envId, { stale, sending: false, exceptId: d.id })}
      />
    </div>
  )
}

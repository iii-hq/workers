// Deploy from this machine (Dep1 what goes, Dep2–4 the check, Dep5 review, Dep6 follow). Only the person
// deploys: every push to an environment's branch, merge and deploy here is one of their clicks. The check
// pushes only the fix branch; the agent fixes and opens pull requests, never deploys.

import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  Checkbox,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  EmptyState,
  Input,
  List,
  ListItem,
  PageMain,
  PageSidebar,
  Panel,
  PanelBody,
  PanelHeader,
  Select,
  Skeleton,
  TerminalStream,
} from '@iii-dev/console-ui'
import { usePaneState } from '@iii-dev/console-ui/hooks'
import { Circle, CircleDot, Info } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { useCloud } from './api'
import { clock, useLaunch } from './asked'
import { CheckStage, MERGE_CODES, MergeConflict } from './CheckStage'
import { handOff, openAgentChat } from './handoff'
import { useActions, useChecking } from './hub'
import {
  activeSameKind,
  ago,
  appFile,
  CHECK_AGAIN,
  COMPOSE_ISSUE,
  cloudStageLabel,
  composeFile,
  deployBusy,
  deployRows,
  deployWord,
  failedStage,
  failureWords,
  isDirectEnv,
  isFailedDeploy,
  minutesLabel,
  needsAction,
  optionalUnset,
  plainDetail,
  plainMessage,
  plural,
  projectName,
  projectUrl,
  RESOURCE_KIND,
  roleOf,
  sendLabel,
  servingDeploy,
  sha7,
  typicalDeployMs,
  usablePr,
} from './model'
import {
  Dot,
  ErrorPanel,
  Fresh,
  LinkButton,
  Mono,
  Notice,
  PathBar,
  Spinner,
  StatusChip,
  StepHeader,
  StepLink,
  useLeave,
  useNow,
} from './parts'
import { type AreaProps, envState, pathOf, type Route, type RouteAt } from './route'
import { Steps, stepStates } from './Steps'
import type { BranchRef, Check, Deploy, Status, WorkerError } from './types'
import { notSet, useValues, ValuesDialog } from './ValuesEditor'

type Act = ReturnType<typeof useActions>
type Health = { status: string; workers: { ready: boolean }[] }
type LogPage = { lines: { worker: string | null; message: string }[] }

/** Deploy errors after which deploying again could deploy twice: Deploys says what happened. */
const LOCKS = new Set([...CHECK_AGAIN, 'PUSH_STILL_RUNNING', 'PUSH_UNKNOWN'])
/** Files `bring_changes` never brings (secrets stay on this machine): never offered. */
const unbringable = (path: string) => /(^|\/)\.env[^/]*$/.test(path)
const PR_CHECKS = { none: 'None', pending: 'Running', passing: 'Passing', failing: 'Failing' } as const
const CHIP_TONE = { running: 'neutral', ready: 'success', failed: 'danger', no_route: 'danger' } as const
const github = (s: Status, path: string) => `https://github.com/${s.project?.repository ?? ''}/${path}`
const span = (from: string, to: string) => Date.parse(to) - Date.parse(from)

const titles = (env: string): Record<DeployStep, { title: string; sub: string }> => ({
  what: { title: 'What goes', sub: `your branch against ${env}` },
  check: { title: 'Check before deploy', sub: 'iii cloud and here; your agent fixes' },
  review: { title: 'Review and deploy', sub: 'you confirm the deploy' },
  follow: { title: 'Live', sub: 'follow it to live' },
})

/** The steps of this screen (the route carries none: the state says where it is, a click on the rail moves it). */
export type DeployStep = 'what' | 'check' | 'review' | 'follow'
const DEPLOY_STEPS: readonly DeployStep[] = ['what', 'check', 'review', 'follow']

/** The step shown: a deployment to follow, the review once the check passed, else what goes. */
function shownStep(r: RouteAt<'deploy'>, s: Status): DeployStep {
  if (r.deploy) return 'follow'
  return s.check?.ready ? 'review' : 'what'
}

/** `check` for a deploy: it runs here, pushes the fix branch (a worktree's only), and keeps to a picked file. */
function checkPayload(s: Status, manifest: string): Record<string, unknown> {
  const worktree = (s.project?.workspace?.mode ?? 'worktree') === 'worktree'
  const switches = manifest !== composeFile(s.project?.environment?.manifest)
  return { run: true, ...(worktree ? { push: true } : {}), ...(switches ? { manifest } : {}) }
}

const summaryText = (c: Check | null | undefined) => {
  const p = c?.plan.summary
  return p ? `${plural(p.workers, 'worker')} in ${plural(p.groups.length, 'group')}` : null
}

/** Deploying from this machine: the step rail beside what goes, the check, the review and the deploy. */
export function DeployArea({ host, status, route, go: goRoute, narrow, paneKey }: AreaProps<RouteAt<'deploy'>>) {
  const act = useActions(host)
  const env = route.env
  const state = envState(status, env)
  const name = status.project?.environments.find((e) => e.id === env)?.name ?? 'the environment'
  // A step the person or the flow moved to (kept per pane, so a reload stays on it); none: the one the state says.
  const [picked, setPicked] = usePaneState<DeployStep | null>(`iii-cloud:deploy-step:${paneKey}:${env}`, null)
  const step = picked ?? shownStep(route, status)
  // Going to this screen again (the check's "Review and deploy") lets the state choose the step.
  const go = (r: Route) => {
    if (r.at === 'deploy' && r.env === env && !r.deploy) setPicked(null)
    goRoute(r)
  }
  const to = (s: DeployStep, deploy?: string) => {
    setPicked(s === 'follow' ? null : s)
    if (s === 'follow' ? deploy : route.deploy)
      goRoute({ at: 'deploy', env, ...(s === 'follow' && deploy ? { deploy } : {}) })
  }
  const checkAgain = () => {
    void act.call('check', 'iii-cloud::check', checkPayload(status, appFile(status).path))
    to('check')
  }
  const deploy = async (payload: Record<string, unknown>) => {
    const d = await act.call<Deploy>('deploy', 'iii-cloud::deploy', payload)
    if (d) to('follow', d.id)
  }
  const stale = !!act.error && LOCKS.has(act.error.code)
  const deploying = act.busy === 'deploy'
  const notBrought = status.project?.workspace?.not_brought ?? []

  let body: ReactNode
  if (state === 'unknown') {
    body = (
      <EmptyState
        title="Not an environment of this project"
        description="It may have been deleted. Pick one from the project's environments."
        action={{ label: `Back to ${projectName(status)}`, onClick: () => go({ at: 'env', env: '' }) }}
      />
    )
  } else if (step === 'what') {
    // Keyed by the selected environment: its app file and choices start over when it changes.
    body = (
      <WhatGoes
        key={status.project?.environment?.id}
        host={host}
        status={status}
        envId={env}
        switching={state === 'switching'}
        // Continue starts the check: this screen's next step.
        go={(r) => (r.at === 'deploy' && r.env === env && !r.deploy ? to('check') : go(r))}
        act={act}
        narrow={narrow}
      />
    )
  } else if (state === 'switching') {
    body = <Skeleton className="iii-cloud-skeleton" />
  } else if (step === 'check') {
    body = <CheckStage host={host} status={status} scenario="change" go={go} narrow={narrow} />
  } else if (step === 'review') {
    body = (
      <Review
        host={host}
        status={status}
        stale={stale}
        deploying={deploying}
        deploy={deploy}
        checkAgain={checkAgain}
        follow={(id) => to('follow', id)}
      />
    )
  } else {
    body = (
      <Follow
        host={host}
        status={status}
        route={route}
        act={act}
        deploying={deploying}
        deploy={deploy}
        checkAgain={checkAgain}
        to={to}
        go={go}
      />
    )
  }

  return (
    <>
      <PageSidebar
        label="Deploy steps"
        narrowMode="drawer"
        narrow={narrow}
        defaultWidth={200}
        header={<span className="iii-cloud-dep-railhead">Deploy to {name}</span>}
      >
        <Steps
          label="Deploy steps"
          steps={stepStates(DEPLOY_STEPS, step, titles(name))}
          onGo={(id) => to(id, id === 'follow' ? route.deploy : undefined)}
        />
      </PageSidebar>
      <PageMain className="iii-cloud-main">
        <PathBar
          path={pathOf(route, status)}
          host={host}
          status={status}
          route={route}
          go={go}
          narrow={narrow}
          rule
          end={<Fresh host={host} status={status} narrow={narrow} />}
        />
        <div className="iii-cloud-body iii-cloud-scroll">
          <div className="iii-cloud-dep">
            {act.error ? (
              <ActionError host={host} status={status} error={act.error} checkAgain={checkAgain} go={go} />
            ) : null}
            {step === 'check' && notBrought.length > 0 && state === 'selected' ? (
              <span className="iii-cloud-dep-sub">
                Not brought: {notBrought.join(', ')} — secrets stay on this machine.
              </span>
            ) : null}
            {body}
          </div>
        </div>
      </PageMain>
    </>
  )
}

/* ── What a call refused, and the way on ─────────────────────────── */

function ActionError({
  host,
  status,
  error,
  checkAgain,
  go,
}: {
  host: Host
  status: Status
  error: WorkerError
  checkAgain: () => void
  go: (r: Route) => void
}) {
  const checking = useChecking(host)
  const p = status.project
  if (MERGE_CODES.has(error.code)) return <MergeConflict host={host} status={status} error={error} />
  if (CHECK_AGAIN.has(error.code)) {
    return (
      <ErrorPanel
        error={error}
        variant="warn"
        action={
          <Button size="sm" variant="primary" disabled={checking} onClick={checkAgain}>
            Check again
          </Button>
        }
      />
    )
  }
  if (error.code === 'PUSH_STILL_RUNNING' || error.code === 'PUSH_UNKNOWN') {
    // PUSH_UNKNOWN is worded in model.ts (its message ends with git's own words, never shown); the
    // page keeps only the file switch it names. PUSH_STILL_RUNNING is the worker's own sentence.
    const stays = error.message.match(/\s(\S+ stays on \S+?)\.?$/)?.[1]
    return (
      <Notice
        variant="warn"
        headline={plainMessage(error)}
        detail={stays ? `${stays}.` : plainDetail(error)}
        action={
          <Button size="sm" variant="secondary" onClick={() => go({ at: 'env', env: p?.environment?.id ?? '' })}>
            See Deployments
          </Button>
        }
      />
    )
  }
  return <ErrorPanel error={error} />
}

/* ── Shared pieces ─────────────────────────────────────────────────── */

function Row({ label, children, sub, end }: { label: string; children: ReactNode; sub?: ReactNode; end?: ReactNode }) {
  return (
    <div className="iii-cloud-dep-row">
      <span className="iii-cloud-dep-key">{label}</span>
      <span className="iii-cloud-dep-val">
        {children}
        {sub ? <span className="iii-cloud-dep-sub">{sub}</span> : null}
      </span>
      {end ?? <span />}
    </div>
  )
}

function Head({ title, sub, chip }: { title: string; sub?: ReactNode; chip?: ReactNode }) {
  return (
    <div className="iii-cloud-dep-head">
      <div className="iii-cloud-dep-title">
        <h2>{title}</h2>
        {sub}
      </div>
      {chip}
    </div>
  )
}

/**
 * Production's confirm: the person types the environment's name, so a deploy there is never one stray click.
 * It stays open, its confirm disabled, until the write's promise settles: a second click sends nothing.
 */
function TypedConfirm({
  open,
  onOpenChange,
  name,
  title,
  description,
  confirmLabel,
  onConfirm,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  name: string
  title: string
  description: string
  confirmLabel: string
  onConfirm: () => Promise<void>
}) {
  const [typed, setTyped] = useState('')
  const [busy, setBusy] = useState(false)
  const close = (next: boolean) => {
    if (busy) return
    setTyped('')
    onOpenChange(next)
  }
  const confirm = async () => {
    setBusy(true)
    try {
      await onConfirm()
    } finally {
      setBusy(false)
      setTyped('')
      onOpenChange(false)
    }
  }
  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent>
        <DialogTitle>{title}</DialogTitle>
        <DialogDescription>{description}</DialogDescription>
        <div className="iii-cloud-dep-typed">
          <span>
            Type <Mono>{name}</Mono> to confirm
          </span>
          <Input
            aria-label={`Type ${name} to confirm`}
            value={typed}
            onChange={setTyped}
            autoComplete="off"
            spellCheck={false}
          />
        </div>
        <div className="iii-cloud-dep-foot">
          <DialogClose asChild>
            <Button variant="secondary" disabled={busy}>
              Cancel
            </Button>
          </DialogClose>
          <Button variant="primary" disabled={busy || typed.trim() !== name} onClick={() => void confirm()}>
            {busy ? (
              <>
                <Spinner />
                {confirmLabel}
              </>
            ) : (
              confirmLabel
            )}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

/* ── Dep1: what goes ───────────────────────────────────────────────── */

type Source = {
  value: string
  label: string
  description?: string
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
    {
      value: '',
      label: `${here ?? sha7(p.checkout.head)} · your folder${mine?.is_env_branch ? ` · what ${env} deploys` : ''}`,
      description: lacks(folderAhead),
      ref: null,
      folder: true,
      ahead: lacks(folderAhead),
    },
    ...[envRef, ...refs.filter((r) => r !== mine && r !== envRef)]
      .filter((r): r is BranchRef => !!r)
      .map((r) => ({
        value: r.name,
        label: r === envRef ? `${r.name} · what ${env} deploys` : r.name,
        description: lacks(r.ahead),
        ref: r,
        folder: false,
        ahead: lacks(r.ahead),
      })),
  ]
  const pick = (folderAhead ?? 0) > 0 || !envRef ? out[0] : out[1]
  pick.default = true
  return out
}

function WhatGoes({
  host,
  status,
  envId,
  switching,
  go,
  act,
  narrow,
}: {
  host: Host
  status: Status
  envId: string
  switching: boolean
  go: (r: Route) => void
  act: Act
  narrow: boolean
}) {
  const checking = useChecking(host)
  const [pick, setPick] = useState(() => appFile(status).path)
  const [picking, setPicking] = useState(false)
  const [bring, setBring] = useState(true)
  // Another environment selects it: something waiting on the person in this one asks first.
  const { leave, dialog } = useLeave(host, status)
  const [source, setSource] = useState<string | null>(null)
  const p = status.project
  if (!p) return null
  const env = switching ? null : p.environment
  const name = p.environments.find((e) => e.id === envId)?.name ?? 'the environment'
  const current = composeFile(env?.manifest)
  const files = status.compose_files ?? []
  const serving = servingDeploy(status, envId)
  const branch = p.checkout.branch
  const notServed = p.checkout.not_served
  const changed = p.checkout.changed_files.filter((f) => !unbringable(f))
  const kept = p.checkout.changed_files.filter(unbringable)
  const production = !!env?.production
  const busy = act.busy === 'use'
  // What the check starts from (a direct or pull-request trial; production always starts from its branch).
  const sources = production ? null : sourcesOf(p, name)
  const chosen = sources ? (sources.find((o) => o.value === source) ?? sources.find((o) => o.default)) : undefined

  const start = async () => {
    const used = await act.call('use', 'iii-cloud::project::use', {
      path: p.path,
      environment_id: envId,
      mode: p.workspace?.mode ?? 'worktree',
      // Production goes through a pull request: its check starts from the environment, never the checkout.
      from: production ? 'environment' : 'checkout',
      ...(chosen?.ref ? { source: chosen.ref.name } : {}),
      bring_changes: bring && changed.length > 0,
    })
    if (used === null) return
    // The check runs on (it waits for the run here); the check step follows it.
    void act.call('check', 'iii-cloud::check', checkPayload(status, pick))
    go({ at: 'deploy', env: envId })
  }

  return (
    <>
      <StepHeader
        title={`What goes to ${name}?`}
        sub={`The worker compared your folder with what ${name} serves now.`}
        back={<StepLink onClick={() => go({ at: 'env', env: envId })}>Cancel</StepLink>}
        forward={
          <Button variant="primary" disabled={busy || checking || !env} onClick={start}>
            {busy ? (
              <>
                <Spinner />
                Preparing the check…
              </>
            ) : (
              'Check before deploy'
            )}
          </Button>
        }
        reason="Nothing goes live before you confirm. Every deploy from this machine is checked first."
      />
      <div className="iii-cloud-dep-group">
        <span className="iii-cloud-dep-legend">Environment</span>
        <List role="radiogroup" aria-label="Environment" className="iii-cloud-dep-envs">
          {/* Trials first, production after them, then a new one. */}
          {[...p.environments]
            .sort((a, b) => Number(!!a.production) - Number(!!b.production))
            .map((e) => {
              const on = e.id === envId
              return (
                <ListItem
                  key={e.id}
                  role="radio"
                  aria-checked={on}
                  className="iii-cloud-choice"
                  leading={on ? <CircleDot /> : <Circle />}
                  label={e.name}
                  description={isDirectEnv(status, e) ? 'pushes, then deploys' : 'opens a pull request'}
                  onClick={on ? undefined : () => leave(e.name, () => go({ at: 'deploy', env: e.id }))}
                />
              )
            })}
          <ListItem
            role="radio"
            aria-checked={false}
            className="iii-cloud-choice"
            leading={<Circle />}
            label="New environment"
            description="a trial for this branch, made in iii cloud's console"
            onClick={() => {
              const url = projectUrl(status)
              if (url) window.open(url, '_blank', 'noreferrer')
            }}
          />
        </List>
      </div>

      {!env ? (
        <Skeleton className="iii-cloud-skeleton" />
      ) : (
        <>
          <Panel>
            <PanelHeader className="iii-cloud-panel-head">Your folder against {name}</PanelHeader>
            <div className="iii-cloud-dep-rows">
              <Row label={`${name} serves`}>
                {serving ? (
                  <span>
                    <Mono>{sha7(serving.commit.sha)}</Mono> {serving.commit.message?.split('\n')[0] ?? ''}{' '}
                    <span className="iii-cloud-faint">
                      · deploy #{serving.number} · {ago(serving.finished_at ?? serving.created_at)}
                    </span>
                  </span>
                ) : (
                  <span>Nothing live yet</span>
                )}
              </Row>
              {sources && chosen ? (
                <Row label="Source" sub={chosen.folder ? null : chosen.ahead}>
                  <Select
                    aria-label="Source"
                    value={chosen.value}
                    options={sources.map(({ value, label, description }) => ({ value, label, description }))}
                    onChange={setSource}
                  />
                  {chosen.folder && notServed?.length ? (
                    <ul className="iii-cloud-dep-commits" aria-label={`Commits ${name} doesn't have`}>
                      {notServed.slice(0, 8).map((c) => (
                        <li key={c.sha}>
                          <Mono>{sha7(c.sha)}</Mono> {c.message}
                        </li>
                      ))}
                      {notServed.length > 8 ? <li>and {notServed.length - 8} more</li> : null}
                    </ul>
                  ) : null}
                </Row>
              ) : (
                <Row
                  label="You're on"
                  sub={
                    production && notServed?.length
                      ? `They aren't in a ${name} check: it starts from ${env.branch}, and changes reach it through a pull request.`
                      : notServed == null
                        ? `The commit ${name} serves couldn't be read, so your commits aren't listed.`
                        : null
                  }
                >
                  <span>
                    {branch ? <Mono>{branch}</Mono> : 'a detached HEAD'}{' '}
                    <span className="iii-cloud-faint">
                      ·{' '}
                      {notServed == null
                        ? `at ${sha7(p.checkout.head)}`
                        : notServed.length
                          ? `${plural(notServed.length, 'commit')} ${name} doesn't have`
                          : `nothing ${name} doesn't have`}
                    </span>
                  </span>
                  {notServed?.length ? (
                    <ul className="iii-cloud-dep-commits" aria-label={`Commits ${name} doesn't have`}>
                      {notServed.slice(0, 8).map((c) => (
                        <li key={c.sha}>
                          <Mono>{sha7(c.sha)}</Mono> {c.message}
                        </li>
                      ))}
                      {notServed.length > 8 ? <li>and {notServed.length - 8} more</li> : null}
                    </ul>
                  ) : null}
                </Row>
              )}
              <Row
                label="App file"
                sub={`The check runs with the file you pick; ${name} switches to it only when you confirm the deploy.`}
              >
                <span className="iii-cloud-row">
                  <Mono>{pick}</Mono>
                  <span className="iii-cloud-dep-sub">
                    {pick === current
                      ? `· the file ${name} runs, set on the environment`
                      : '· changes with this deploy'}
                  </span>
                  {files.length > 1 ? (
                    <Button size="sm" variant="secondary" aria-expanded={picking} onClick={() => setPicking(!picking)}>
                      {picking ? 'Done' : 'Change'}
                    </Button>
                  ) : null}
                </span>
                {picking ? (
                  <List role="radiogroup" aria-label="Compose files in this folder" className="iii-cloud-dep-files">
                    {files.map((f) => {
                      const on = f.path === pick
                      const note = [
                        f.workers != null ? plural(f.workers, 'worker') : null,
                        f.path === current ? `what ${name} runs now` : null,
                        f.choosable ? null : (COMPOSE_ISSUE[f.issue ?? ''] ?? f.issue),
                      ]
                        .filter(Boolean)
                        .join(' · ')
                      return (
                        <ListItem
                          key={f.path}
                          role="radio"
                          aria-checked={on}
                          selected={on}
                          disabled={!f.choosable}
                          leading={on ? <CircleDot /> : <Circle />}
                          label={<Mono>{f.path}</Mono>}
                          description={note}
                          onClick={() => setPick(f.path)}
                        />
                      )
                    })}
                  </List>
                ) : null}
              </Row>
              <ValuesRow host={host} status={status} pick={pick} name={name} narrow={narrow} />
              {changed.length || kept.length ? (
                <Row
                  label="Not committed"
                  sub={kept.length ? `Not brought: ${kept.join(', ')} — secrets stay on this machine.` : null}
                >
                  {changed.length ? (
                    <>
                      <span>
                        {plural(changed.length, 'file')} changed{' '}
                        <span className="iii-cloud-faint">
                          · {changed.slice(0, 3).join(', ')}
                          {changed.length > 3 ? ` and ${changed.length - 3} more` : ''}
                        </span>
                      </span>
                      <Checkbox
                        checked={bring}
                        onChange={(e) => setBring(e.target.checked)}
                        label={
                          production
                            ? 'Include them in the check.'
                            : `Include them. They come from your folder and go on top of ${chosen?.ref?.name ?? branch ?? 'your folder'} as one commit you can review.`
                        }
                      />
                    </>
                  ) : null}
                </Row>
              ) : null}
            </div>
          </Panel>
          {isDirectEnv(status, env) ? (
            <p className="iii-cloud-dep-inset">
              <Info aria-hidden />
              <span>
                {name} deploys whatever is on <Mono>{env.branch}</Mono>. The check starts from it and adds what you pick
                here; after the check passes, your deploy pushes the checked commit there, never forced, so nothing on
                that branch is lost.
              </span>
            </p>
          ) : (
            <p className="iii-cloud-dep-inset">
              <Info aria-hidden />
              <span>
                {name} deploys what is merged into <Mono>{env.branch}</Mono>.{' '}
                {production
                  ? `The check starts from ${env.branch}, not your branch. After it passes, your agent opens a pull request and you merge it here to deploy.`
                  : 'After the check passes, your agent opens a pull request with your commits and its fixes; you merge it here to deploy.'}
              </span>
            </p>
          )}
        </>
      )}
      {dialog}
    </>
  )
}

/**
 * Dep1's Values row: how many names the folder's file reads have no value in the environment, and "Set values".
 * It reads the person's folder (`from: 'checkout'`) with the file picked above, again when the pick changes.
 */
function ValuesRow({
  host,
  status,
  pick,
  name,
  narrow,
}: {
  host: Host
  status: Status
  pick: string
  name: string
  narrow: boolean
}) {
  const read = useValues(host, status, { manifest: pick, from: 'checkout' })
  const [open, setOpen] = useState(false)
  const v = read.data
  if (!v && !read.error) return null
  const unset = v?.items.filter(notSet).length ?? 0
  const skipped = v?.items.filter((i) => notSet(i) && i.skipped).length ?? 0
  // Only a required name (`${X}`) without a value is worth a strong word; an optional one defaults to empty.
  const required = v?.items.filter((i) => notSet(i) && i.need === 'required').map((i) => i.name) ?? []
  return (
    <Row label="Values">
      <span className="iii-cloud-row">
        {!v ? (
          <span>Couldn't read the names your folder's file reads.</span>
        ) : v.cloud === 'unreadable' ? (
          <span>Couldn't read {name}'s values in iii cloud.</span>
        ) : required.length ? (
          <span>
            {plural(required.length, 'required name')} your folder's <Mono>{pick}</Mono> reads{' '}
            {required.length === 1 ? "isn't" : "aren't"} set in {name}: <Mono>{required.join(', ')}</Mono>
          </span>
        ) : unset ? (
          <span className="iii-cloud-faint">
            {/* "5 optional names have no value in dev; 2 skipped": the skipped are counted in, said once. */}
            {skipped
              ? `${plural(unset, 'optional name')} ${unset === 1 ? 'has' : 'have'} no value in ${name}; ${skipped} skipped`
              : optionalUnset(unset, name)}
          </span>
        ) : (
          <span>
            All {v.items.length} are set in {name}
          </span>
        )}
        {v && (unset || v.cloud === 'unreadable') ? (
          <Button size="sm" variant="secondary" onClick={() => setOpen(true)}>
            Set values
          </Button>
        ) : null}
      </span>
      {open ? (
        <ValuesDialog
          host={host}
          status={status}
          manifest={pick}
          from="checkout"
          narrow={narrow}
          onClose={() => setOpen(false)}
          onSaved={read.refetch}
        />
      ) : null}
    </Row>
  )
}

/* ── Dep5: review and deploy ───────────────────────────────────────── */

/** The checks of this effort: the current one back to (not including) the environment's previous ready one. */
function checkGroup(s: Status, c: Check): { count: number; since: string } | null {
  const mine = (s.checks ?? []).filter((x) => x.environment_id === c.environment_id)
  const at = mine.findIndex((x) => x.id === c.id)
  if (at < 0) return null
  // ponytail: an effort is "since the previous ready check"; split by handoff_id if efforts overlap.
  let end = at + 1
  while (end < mine.length && !mine[end].ready) end++
  return { count: end - at, since: mine[end - 1].checked_at }
}

function Review({
  host,
  status,
  stale,
  deploying,
  deploy,
  checkAgain,
  follow,
}: {
  host: Host
  status: Status
  stale: boolean
  /** This page's deploy call is in flight. */
  deploying: boolean
  deploy: (payload: Record<string, unknown>) => Promise<void>
  checkAgain: () => void
  follow: (deployId: string) => void
}) {
  const [confirming, setConfirming] = useState(false)
  const launching = useLaunch()?.kind === 'launching'
  const checking = useChecking(host)
  const p = status.project
  const env = p?.environment
  const c = status.check
  if (!p || !env || !c) return null
  const direct = isDirectEnv(status, env)
  const pr = direct ? null : usablePr(status)
  const file = appFile(status)
  const group = checkGroup(status, c)
  const { resources, overrides } = c.plan.confirmations
  const role = roleOf(status)
  const serving = servingDeploy(status, env.id)
  const typical = typicalDeployMs(status, env.id)
  const project = p.cloud_project?.name ?? p.repository

  // Whose commits (newest first): the agent's down to `from_head`, the checkout's from it. No `from_head`: all the agent's.
  const commits = c.commits ?? []
  const from = p.workspace?.from_head
  const at = from ? commits.findIndex((x) => x.sha === from) : -1
  const agent = !from ? commits.length : at >= 0 ? at : from === c.base ? commits.length : null
  const whose =
    agent === null
      ? null
      : [commits.length - agent ? `${commits.length - agent} of yours` : null, agent ? `${agent} by your agent` : null]
          .filter(Boolean)
          .join(', ')

  const release = c.run?.stages.find((s) => s.name === 'release' && s.status === 'passed' && s.detail)
  const since = group?.since ?? c.checked_at
  const saved = status.secrets.filter(
    (r) =>
      r.environment_id === env.id &&
      r.state === 'set' &&
      !!r.resolved_at &&
      Date.parse(r.resolved_at) >= Date.parse(since),
  )
  const newest = status.deploys.find((d) => d.environment_id === env.id)
  const changed =
    newest && newest.check_id !== c.id && Date.parse(newest.created_at) > Date.parse(c.checked_at) ? newest : null
  // Any deploy of the environment still going (queued, building, held): deploying now would replace it.
  const busy = status.deploys.find((d) => d.environment_id === env.id && d.outcome === 'running')
  const locked = !c.ready || stale || deploying || !!busy || !!changed || (!direct && (!pr || pr.main_moved))
  const base = pr?.base ?? env.branch
  const label = direct ? `Deploy ${sha7(c.commit)} to ${env.name}` : `Merge to ${base} and deploy to ${env.name}`
  const payload = pr ? { check_id: c.id, pr: pr.number } : { check_id: c.id }
  const ships = direct || !!pr

  return (
    <>
      <StepHeader
        title={`Ready to deploy to ${env.name}`}
        chip={<StatusChip tone="success">Passed</StatusChip>}
        sub={
          <p>
            Commit <Mono>{sha7(c.commit)}</Mono> passed on iii cloud{c.run?.status === 'passed' ? ' and here' : ''} at{' '}
            {clock(c.checked_at)}
            {group && group.count > 1
              ? ` · ${group.count} checks, ${minutesLabel(span(group.since, c.checked_at))}`
              : ''}
          </p>
        }
        forward={
          <Button
            variant="primary"
            disabled={locked}
            onClick={() => (env.production ? setConfirming(true) : void deploy(payload))}
          >
            {env.production ? `${label}…` : label}
          </Button>
        }
        reason={typical ? `About ${minutesLabel(typical)}.` : null}
      />
      {busy ? (
        <Notice
          variant="info"
          {...deployBusy(status, busy, env.name)}
          action={
            <Button size="sm" variant="primary" onClick={() => follow(busy.id)}>
              Follow #{busy.number}
            </Button>
          }
        />
      ) : null}
      {changed ? (
        <Notice
          variant="warn"
          headline={`${env.name} changed after the check`}
          detail={`Deploy #${changed.number} (${sha7(changed.commit.sha)}) started after this check. Check on top of it before you deploy.`}
          action={
            <Button size="sm" variant="primary" disabled={checking} onClick={checkAgain}>
              Check on top of {sha7(changed.commit.sha)}
            </Button>
          }
        />
      ) : null}
      {pr?.main_moved ? (
        <Notice
          variant="warn"
          headline={`${pr.base} moved since the check`}
          detail="Check again before you deploy: the merge would bring in what the check didn't see."
          action={
            <Button size="sm" variant="primary" disabled={checking} onClick={checkAgain}>
              Check again
            </Button>
          }
        />
      ) : null}
      {env.production && role && !/^(owner|admin)$/i.test(role) ? (
        <Notice
          variant="warn"
          headline={`Deploying to ${env.name} takes Owner or Admin`}
          detail={`You are ${role}. iii cloud decides; if it refuses, its answer shows here.`}
        />
      ) : null}
      <Panel>
        <PanelHeader className="iii-cloud-panel-head">What the deploy does</PanelHeader>
        <div className="iii-cloud-dep-rows">
          <Row
            label="Code"
            sub={whose}
            end={
              <LinkButton href={github(status, c.base ? `compare/${c.base}...${c.commit}` : `commit/${c.commit}`)}>
                See changes
              </LinkButton>
            }
          >
            <span>
              {c.commits
                ? commits.length
                  ? `${plural(commits.length, 'commit')} ${env.name} doesn't have`
                  : `Nothing ${env.name} doesn't have`
                : `Commit ${sha7(c.commit)}`}
            </span>
          </Row>
          <Row
            label="App file"
            sub={[summaryText(c), file.note ? `${file.note}.` : null].filter(Boolean).join(' · ') || null}
            end={<LinkButton href={github(status, `blob/${c.commit}/${file.path}`)}>See the file</LinkButton>}
          >
            <Mono>{file.path}</Mono>
          </Row>
          {resources.length || release ? (
            <Row
              label="Database"
              sub={[
                release ? 'It ran here in the check.' : null,
                env.production && resources.length
                  ? `Approving resources in ${env.name} takes Owner or Admin. You are ${role ?? 'a member'}.`
                  : null,
              ]
                .filter(Boolean)
                .join(' ')}
            >
              {resources.map((r) => (
                <span key={`${r.kind}:${r.alias}`}>
                  Creates a shared {RESOURCE_KIND[r.kind] ?? r.kind} for <Mono>{r.binding ?? r.alias}</Mono>
                </span>
              ))}
              {release ? (
                <span>
                  Release command <Mono>{release.detail}</Mono> runs before the new version starts
                </span>
              ) : null}
            </Row>
          ) : null}
          <Row label="Secrets">
            <span>
              {saved.length ? (
                <>
                  {saved.length} new: <Mono>{saved.map((r) => r.name).join(', ')}</Mono>, saved by you
                </>
              ) : (
                'No new secrets'
              )}
            </span>
          </Row>
          <Row
            label="Console settings"
            sub={
              overrides.length
                ? `${overrides.map((o) => o.setting).join(', ')} now ${overrides.length === 1 ? 'comes' : 'come'} from ${file.path}.`
                : null
            }
          >
            <span>
              {overrides.length ? `Changes ${plural(overrides.length, 'setting')} the console had` : 'No change'}
            </span>
          </Row>
          <Row
            label="How it ships"
            sub={
              [
                ships
                  ? serving
                    ? `${env.name} serves #${serving.number} until the new deploy is live.`
                    : `Nothing is live on ${env.name} yet.`
                  : null,
                pr ? `${pr.head} → ${pr.base}, opened by your agent, with your GitHub account.` : null,
                pr
                  ? `GitHub checks: ${PR_CHECKS[pr.checks]}.${pr.main_moved ? '' : ` ${pr.base} has not moved since the check.`}`
                  : null,
                ships ? "Only you can confirm this; your agent can't." : null,
                ships
                  ? 'Pinned to the plan the check saw: if it plans differently, nothing is deployed and you see why.'
                  : null,
              ]
                .filter(Boolean)
                .join(' ') || null
            }
            end={
              pr ? (
                <LinkButton href={pr.url}>Open on GitHub</LinkButton>
              ) : direct ? undefined : activeSameKind(status, 'pull_request') ? (
                <Button size="sm" variant="secondary" onClick={() => openAgentChat(host, status)}>
                  Open your agent's chat
                </Button>
              ) : (
                <Button
                  size="sm"
                  variant="primary"
                  disabled={launching || !host.chat?.compose}
                  onClick={() => handOff(host, status, 'pull_request')}
                >
                  {sendLabel(status, 'pull_request', 'Ask your agent to open it')}
                </Button>
              )
            }
          >
            <span>
              {direct ? (
                <>
                  Pushes <Mono>{sha7(c.commit)}</Mono> to <Mono>{env.branch}</Mono>, then starts a deploy
                </>
              ) : pr ? (
                <>
                  Merges pull request #{pr.number}, {pr.title}, into <Mono>{pr.base}</Mono>, then deploys
                </>
              ) : (
                <>
                  Your agent opens a pull request from <Mono>{p.workspace?.branch ?? 'its fix branch'}</Mono> into{' '}
                  <Mono>{env.branch}</Mono>; merging it here deploys
                </>
              )}
            </span>
          </Row>
        </div>
      </Panel>
      <TypedConfirm
        open={confirming}
        onOpenChange={setConfirming}
        name={env.name}
        title={
          pr
            ? `Merge pull request #${pr.number} to ${pr.base} and deploy ${project} to ${env.name}?`
            : `Deploy ${sha7(c.commit)} of ${project} to ${env.name}?`
        }
        description={
          pr
            ? `This merges pull request #${pr.number} (${pr.head}) into ${pr.base}. The merge starts the deploy in iii cloud, and this page follows it.`
            : `This pushes ${sha7(c.commit)} to ${env.branch} and starts the deploy in iii cloud; this page follows it.`
        }
        confirmLabel={pr ? 'Merge and deploy' : 'Deploy'}
        onConfirm={() => deploy(payload)}
      />
    </>
  )
}

/* ── Dep6: follow it to live ──────────────────────────────────────── */

function Follow({
  host,
  status,
  route,
  act,
  deploying,
  deploy,
  checkAgain,
  to,
  go,
}: {
  host: Host
  status: Status
  route: RouteAt<'deploy'>
  act: Act
  deploying: boolean
  deploy: (payload: Record<string, unknown>) => Promise<void>
  checkAgain: () => void
  to: (s: DeployStep, deploy?: string) => void
  go: (r: Route) => void
}) {
  const now = useNow(15_000)
  const launching = useLaunch()?.kind === 'launching'
  const checking = useChecking(host)
  const [confirming, setConfirming] = useState(false)
  const envId = route.env
  // The deploy in the route, only if it is this environment's; none asked (or not known yet): the newest here.
  const asked = status.deploys.find((x) => x.id === route.deploy)
  const elsewhere = !!asked && asked.environment_id !== envId
  const d = elsewhere ? undefined : (asked ?? status.deploys.find((x) => x.environment_id === envId))
  const running = d?.outcome === 'running' && !needsAction(d)
  const live = d?.outcome === 'ready'
  // The worker reads a deploy's log only once it ends: while it runs, the page reads the cloud's.
  const logs = useCloud<LogPage>(host, status, envId, running && d ? `/logs?deployment_id=${d.id}` : null, {
    pollMs: 5000,
  })
  const health = useCloud<Health>(host, status, envId, live ? '/health' : null)
  const env = status.project?.environment
  if (!env) return null
  if (elsewhere) {
    return (
      <EmptyState
        title="Deployment not found"
        description={`It isn't a deployment of ${env.name}. Its own environment has it.`}
        action={{ label: 'Open deployments', onClick: () => go({ at: 'env', env: envId }) }}
      />
    )
  }
  if (!d) {
    return (
      <EmptyState
        title="Nothing to follow yet"
        description={`No deploy of ${env.name} is known here. Review the check and deploy it.`}
        action={{ label: 'Review and deploy', onClick: () => to('review') }}
      />
    )
  }
  const sha = sha7(d.commit.sha)
  const failed = isFailedDeploy(d)
  const held = needsAction(d)
  const prev = status.deploys.find(
    (x) =>
      x.environment_id === envId &&
      x.id !== d.id &&
      x.outcome === 'ready' &&
      Date.parse(x.created_at) < Date.parse(d.created_at),
  )
  const c = status.check
  // The deploy is of this check: its id when the deploy names one, else the same commit.
  const ours = !!c && (d.check_id ? d.check_id === c.id : d.commit.sha === c.commit)
  const confirmable = !!c?.ready && ours
  const stage = d.failure ? cloudStageLabel(d.failure.stage) : failedStage(d)
  const title = held
    ? `Deploy #${d.number} needs your action`
    : running
      ? `Deploying ${sha} to ${env.name}`
      : live
        ? `${env.name} is serving ${sha}`
        : failed
          ? `Deploy #${d.number} failed at ${stage}`
          : `Deploy #${d.number}: ${deployWord(d)}`
  const sub = held
    ? `Deploy #${d.number} · iii cloud holds it until you confirm`
    : running
      ? `Deploy #${d.number} · started ${clock(d.created_at)} · ${minutesLabel(now - Date.parse(d.created_at))} so far`
      : live && d.finished_at
        ? `Deploy #${d.number} · live at ${clock(d.finished_at)} · took ${minutesLabel(span(d.created_at, d.finished_at))}`
        : failed
          ? prev
            ? `${env.name} still serves #${prev.number} · nothing changed for your users`
            : `Nothing was live on ${env.name} before it`
          : `Deploy #${d.number} · ${ago(d.finished_at ?? d.created_at)}`
  const tone = held ? 'warning' : (CHIP_TONE[d.outcome as keyof typeof CHIP_TONE] ?? 'neutral')
  const checkedSha = status.checks?.find((x) => x.id === d.check_id)?.commit ?? (ours ? c?.commit : null)
  const workers = health.data?.workers ?? []
  const { resources, overrides } = (ours ? c?.plan.confirmations : null) ?? { resources: [], overrides: [] }
  const approves = [
    ...resources.map((r) => `a shared ${RESOURCE_KIND[r.kind] ?? r.kind} for ${r.binding ?? r.alias}`),
    overrides.length ? `${plural(overrides.length, 'setting')} from the app file` : null,
  ].filter(Boolean)
  const confirmPayload = { check_id: c?.id, deployment_id: d.id }
  // Newest first from the cloud; the last lines, oldest first.
  const logText = (logs.data?.lines ?? [])
    .slice(0, 12)
    .reverse()
    .map((l) => (l.worker ? `${l.worker}: ${l.message}` : l.message))
    .join('\n')

  return (
    <>
      <Head
        title={title}
        sub={
          <>
            <p>{sub}</p>
            {ours ? (
              <p className="iii-cloud-dep-sub">
                App file <Mono>{appFile(status).path}</Mono>
                {summaryText(c) ? ` · ${summaryText(c)}` : ''}
              </p>
            ) : null}
            {running ? (
              <p className="iii-cloud-dep-sub">
                It keeps going if you switch projects; the pill at the top brings you back.
              </p>
            ) : null}
          </>
        }
        chip={
          <span aria-live="polite">
            <StatusChip tone={tone} live={running}>
              {deployWord(d)}
            </StatusChip>
          </span>
        }
      />
      {running && status.connection.status === 'expired' ? (
        <Notice
          variant="warn"
          headline="Signed out while it deploys"
          detail="The deploy goes on in iii cloud. Sign in again to follow it here."
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
      {held ? (
        <Notice
          variant="warn"
          headline="Needs your action"
          detail={`iii cloud holds this deploy until you approve what it creates and changes${approves.length ? `: ${approves.join(', ')}` : ''}.${confirmable ? '' : ` No passed check here matches deploy #${d.number}. Check again before you deploy.`}`}
          action={
            <span className="iii-cloud-row">
              <Button
                size="sm"
                variant="primary"
                disabled={!confirmable || deploying}
                onClick={() => (env.production ? setConfirming(true) : void deploy(confirmPayload))}
              >
                Confirm deploy
              </Button>
              {confirmable ? null : (
                <Button size="sm" variant="secondary" disabled={checking} onClick={checkAgain}>
                  Check again
                </Button>
              )}
            </span>
          }
        />
      ) : null}
      <Panel>
        <PanelHeader className="iii-cloud-panel-head iii-cloud-dep-ph">
          <span>Stages</span>
          <LinkButton href={d.console_url}>Open in iii cloud</LinkButton>
        </PanelHeader>
        <List aria-label="Deploy stages">
          {deployRows(d).map((r) => (
            <ListItem
              as="div"
              key={r.key}
              leading={<Dot tone={r.tone} />}
              label={
                <span className={r.tone === 'edge' ? 'iii-cloud-faint' : undefined}>
                  {r.label}
                  {r.detail ? `: ${r.detail}` : null}
                </span>
              }
              trailing={<span className="iii-cloud-mono iii-cloud-faint">{r.time}</span>}
            />
          ))}
        </List>
      </Panel>

      {failed ? (
        <Panel>
          <PanelHeader className="iii-cloud-panel-head iii-cloud-dep-ph">
            <span>{d.checked_ready ? 'Something only the cloud could see' : 'What went wrong'}</span>
            {d.checked_ready && checkedSha ? (
              <span className="iii-cloud-dep-sub">the check here passed on {sha7(checkedSha)}</span>
            ) : null}
          </PanelHeader>
          <PanelBody className="iii-cloud-dep-cause">
            <p>
              {d.failure?.title ??
                `The deploy failed at ${failedStage(d)}${d.error ? `: ${failureWords(d.error)}` : ''}`}
            </p>
            {(d.failure?.hint ?? d.error?.hint) ? (
              <p className="iii-cloud-dep-sub">{d.failure?.hint ?? d.error?.hint}</p>
            ) : null}
            <TerminalStream label="log" text={(d.failure?.log_tail ?? d.log_tail).join('\n')} />
            <div className="iii-cloud-dep-actions">
              {activeSameKind(status, 'deploy_fix') ? (
                <Button variant="secondary" onClick={() => openAgentChat(host, status)}>
                  Open your agent's chat
                </Button>
              ) : (
                <Button
                  variant="primary"
                  disabled={launching || !host.chat?.compose}
                  onClick={async () => {
                    if (await handOff(host, status, 'deploy_fix', d)) to('check')
                  }}
                >
                  {sendLabel(status, 'deploy_fix', 'Fix with your agent')}
                </Button>
              )}
              <span className="iii-cloud-dep-sub">
                Your agent gets these log lines, and its fix goes through Check before deploy again.
              </span>
            </div>
          </PanelBody>
        </Panel>
      ) : null}

      {live ? (
        <>
          <Panel aria-label="After the deploy">
            {env.url ? (
              <PanelHeader className="iii-cloud-panel-head iii-cloud-dep-ph">
                <Mono>{env.url}</Mono>
                <LinkButton href={env.url}>Open</LinkButton>
              </PanelHeader>
            ) : (
              <PanelHeader className="iii-cloud-panel-head">After the deploy</PanelHeader>
            )}
            <List aria-label="Health">
              {health.data ? (
                <ListItem
                  as="div"
                  leading={<Dot tone={health.data.status === 'healthy' ? 'ok' : 'warn'} />}
                  label={`iii cloud says it's ${health.data.status}`}
                  trailing={
                    <span className="iii-cloud-dep-sub">
                      {workers.filter((w) => w.ready).length} of {plural(workers.length, 'worker')} up
                    </span>
                  }
                />
              ) : health.error ? (
                <ListItem as="div" leading={<Dot tone="ghost" />} label={health.error.message} />
              ) : (
                <Skeleton className="iii-cloud-dep-skel" />
              )}
            </List>
          </Panel>
          <div className="iii-cloud-dep-actions">
            <Button variant="primary" onClick={() => go({ at: 'env', env: envId })}>
              Back to {env.name}
            </Button>
            <Button variant="secondary" onClick={() => go({ at: 'env', env: envId, deploy: d?.id })}>
              Open deployments
            </Button>
            {prev ? (
              <span className="iii-cloud-dep-sub">
                If something's off, roll back to #{prev.number} in iii cloud's console.
              </span>
            ) : null}
          </div>
        </>
      ) : null}

      {running ? (
        <>
          {logText ? <TerminalStream label="Live log" text={logText} /> : null}
          <span className="iii-cloud-dep-sub">
            {prev ? `${env.name} keeps serving #${prev.number} until #${d.number} is live. ` : ''}You can leave this
            page; the deploy goes on. If it fails, Fix with your agent gets its log lines.
          </span>
        </>
      ) : null}

      <TypedConfirm
        open={confirming}
        onOpenChange={setConfirming}
        name={env.name}
        title={`Confirm deploy #${d.number} to ${env.name}?`}
        description={`This approves what deploy #${d.number} creates and changes, so iii cloud lets it go on.`}
        confirmLabel="Confirm deploy"
        onConfirm={() => deploy(confirmPayload)}
      />
    </>
  )
}

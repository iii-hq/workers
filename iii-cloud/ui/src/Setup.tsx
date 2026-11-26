import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  Checkbox,
  ConfirmDialog,
  DirectoryPicker,
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
import { useCopyFlash, usePaneState } from '@iii-dev/console-ui/hooks'
import { Check, ChevronDown, ChevronRight, Circle, CircleDot, Info } from 'lucide-react'
import { type ReactNode, useEffect, useState } from 'react'
import { AgentRow } from './AgentRow'
import { cloudWrite, useCloud } from './api'
import { clock, useLaunch } from './asked'
import { CheckStage } from './CheckStage'
import { ConnectView } from './ConnectView'
import { type Created, NewEnvironmentForm, NewProjectForm } from './CreateForms'
import { FailureCard } from './FailureCard'
import { handOff } from './handoff'
import { hubOf, useActions, useChecking } from './hub'
import {
  appFile,
  CHECK_AGAIN,
  COMPOSE_ISSUE,
  checklist,
  composeFile,
  deployRows,
  isDirectEnv,
  isFailedDeploy,
  minutesLabel,
  needsAction,
  needsConnect,
  plural,
  projectName,
  RESOURCE_KIND,
  sendLabel,
  servingDeploy,
  sha7,
  signInRequired,
  typicalDeployMs,
  usablePr,
  whoLine,
  withAppFile,
} from './model'
import { Dot, ErrorPanel, LinkButton, Mono, Notice, PathBar, StatusChip, StepHeader, StepLink, useNow } from './parts'
import { type AreaProps, pathOf, type Route, type RouteAt } from './route'
import { Steps, stepStates } from './Steps'
import type { CloudProjectChoice, ComposeFile, Deploy, Project, Status, WorkerError } from './types'
import { ValuesEditor } from './ValuesEditor'

/**
 * The steps inside Setup until the redesigned areas take it over: the route carries only `signin` and `confirm`
 * (Your code and Where it runs), the rest is the check and the deploy, which the Check and deploy area will
 * own. ponytail: these stay as Setup's own state, derived from the status, so first-time setup keeps working.
 */
type SetupStep = 'signin' | 'code' | 'target' | 'values' | 'check' | 'ship' | 'live'
const SETUP_STEPS: readonly SetupStep[] = ['signin', 'code', 'target', 'values', 'check', 'ship', 'live']

function flowStep(s: Status): SetupStep {
  if (needsConnect(s) || signInRequired(s)) return 'signin'
  if (!s.project) return 'code'
  if (!s.project.workspace) return 'target'
  const latest = s.deploys.find((d) => d.environment_id === s.project?.environment?.id)
  if (latest?.outcome === 'ready') return 'live'
  if (latest?.outcome === 'running' || (latest && needsAction(latest)) || s.check?.ready) return 'ship'
  return 'check'
}

/** The step the status is at; an asked step is honored only when it is that one or an earlier one. */
function setupStep(s: Status, asked?: SetupStep): SetupStep {
  const at = flowStep(s)
  return asked && SETUP_STEPS.indexOf(asked) <= SETUP_STEPS.indexOf(at) ? asked : at
}

const TITLES: Record<SetupStep, { title: string; sub: string }> = {
  signin: { title: 'Start and sign in', sub: 'once, in your browser' },
  code: { title: 'Your code', sub: 'folder and app file' },
  target: { title: 'Where it runs', sub: 'project and environment' },
  values: { title: 'Values', sub: 'what your app reads' },
  check: { title: 'Check before deploy', sub: 'iii cloud and here; your agent fixes' },
  ship: { title: 'Review and deploy', sub: 'you confirm the deploy' },
  live: { title: 'Live', sub: 'address and health' },
}

/** What the person picked on the way to the check (steps 2 and 3); it lives as long as the area. */
type Draft = {
  path: string
  /** The app file picked in step 2; null: the environment's own. */
  manifest: string | null
  bring: boolean
  mode: 'worktree' | 'checkout'
  cp: string | null
  env: string | null
}
type StepProps = {
  host: Host
  /** What the steps read: in add mode the candidate folder stands in for the project, with no check yet. */
  status: Status
  go: (r: Route) => void
  /** Another setup step, keeping add mode and the new project form. */
  step: (s: SetupStep) => void
  /** Where Cancel goes: Start while adding, the project's home once one is in use; none on the first run. */
  cancel: Route | null
  adding: boolean
  /** The project in use (`status.project` outside add mode). */
  selected: Status['project']
  /** `project::resolve` a folder for the steps (and a team); the step stays where it is. */
  read: (path: string, team?: string) => void
  actions: ReturnType<typeof useActions>
  draft: Draft
  edit: (d: Partial<Draft>) => void
}

/** Setting up a project, from sign-in to live (Onb1–Onb7); step 4 is the shared check. */
export function SetupArea({ host, status, route, go, narrow, workingDir, paneKey }: AreaProps<RouteAt<'setup'>>) {
  const actions = useActions(host)
  const adding = !!route.adding
  // Adding a project leaves the one in use selected: the steps read the folder `resolve` read beside it (the
  // candidate), once this area read it with success, and only for the folder picked: one left from an earlier
  // visit, or kept by a resolve that failed, is another folder's.
  const [fresh, setFresh] = useState(false)
  const [draft, setDraft] = useState<Draft>(() => ({
    path:
      (adding
        ? (status.candidate?.path ?? workingDir ?? (route.newProject ? status.project?.path : null))
        : (status.project?.path ?? workingDir)) ?? '',
    manifest: null,
    bring: false,
    mode: (adding ? null : status.project?.workspace?.mode) ?? 'worktree',
    cp: null,
    env: null,
  }))
  const candidate = fresh && status.candidate?.path === draft.path ? status.candidate : null
  const view: Status = adding ? { ...status, project: candidate, check: null, compose_files: [] } : status
  const at = setupStep(view)
  // The step the person moved to (the rail, a Back, Continue); else the route's (`signin`, or a new cloud project).
  const [local, setLocal] = usePaneState<SetupStep | null>(`iii-cloud:setup-step:${paneKey}`, null)
  const asked =
    route.step === 'signin' ? 'signin' : (local ?? (route.newProject ? 'target' : route.step ? 'code' : undefined))
  const step = setupStep(view, asked)
  const toStep = (s: SetupStep) => setLocal(s)
  const cancel: Route | null = adding ? { at: 'start' } : status.project?.workspace ? { at: 'env', env: '' } : null
  const edit = (d: Partial<Draft>) => setDraft((x) => ({ ...x, ...d }))
  const read = (path: string, team?: string) => {
    edit({ path, manifest: null, cp: null, env: null })
    setFresh(false)
    void actions
      .call<Project>('resolve', 'iii-cloud::project::resolve', { path, ...(team ? { team_id: team } : {}) })
      .then(async (out) => {
        if (out === null) return
        // The worker reads the repository's top folder; the candidate arrives with the status (no event).
        if (out?.path) edit({ path: out.path })
        await hubOf(host).refresh(false)
        setFresh(true)
      })
  }
  // Adding reads its folder again on arrival; a new cloud project may be for the folder in use.
  useEffect(() => {
    if (adding && draft.path && (route.newProject || draft.path !== status.project?.path)) read(draft.path)
  }, [])
  const props: StepProps = {
    host,
    status: view,
    go,
    step: toStep,
    cancel,
    adding,
    selected: status.project,
    read,
    actions,
    draft,
    edit,
  }
  const body =
    step === 'signin' ? (
      <SignIn {...props} />
    ) : step === 'code' ? (
      <Code {...props} workingDir={workingDir} />
    ) : step === 'target' ? (
      <Target {...props} newProject={route.newProject} />
    ) : step === 'values' ? (
      <ValuesStep {...props} narrow={narrow} />
    ) : step === 'check' ? (
      <CheckStage
        host={host}
        status={status}
        scenario="setup"
        // The check's Review and deploy is this screen's own next step until the deploy screen takes setup over.
        go={(r) => (r.at === 'deploy' && !r.deploy ? toStep('ship') : go(r))}
        narrow={narrow}
      />
    ) : step === 'ship' ? (
      <Ship {...props} />
    ) : (
      <Live {...props} />
    )
  // Target and Ship show the error beside the action it is about.
  const own = step === 'target' || step === 'ship'
  // The step shown is the current one; the status's step marks what is done (and links back while an earlier one shows).
  // A deploy of the setup environment going on: step 6 says which (it shows that deploy, route.ts derivedStep).
  const deploying = adding
    ? null
    : status.deploys.find(
        (d) => d.environment_id === status.project?.environment?.id && d.outcome === 'running' && !needsAction(d),
      )
  const titles = deploying ? { ...TITLES, ship: { ...TITLES.ship, sub: `deploying #${deploying.number}` } } : TITLES
  const steps = stepStates(SETUP_STEPS, step, titles, at)
  const where = [status.project?.cloud_project?.name, status.project?.environment?.name].filter(Boolean).join(' · ')
  return (
    <>
      <PageSidebar
        label="Setup steps"
        narrowMode="drawer"
        narrow={narrow}
        defaultWidth={200}
        header={<span className="iii-cloud-dep-railhead">{`Set up ${projectName(view)}`.trim()}</span>}
      >
        {/* Each step goes to itself; the path bar is the way out. */}
        <Steps label="Setup steps" steps={steps} onGo={toStep} />
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
          end={narrow && where ? <span className="iii-cloud-setup-where">{where}</span> : undefined}
        />
        {narrow ? <StepDots steps={steps} /> : null}
        <div className="iii-cloud-setup" data-step={step}>
          {actions.error && !own ? <ErrorPanel error={actions.error} /> : null}
          {body}
        </div>
      </PageMain>
    </>
  )
}

/** A phone's rail in one line: a dot per step, then "Values  step 4 of 7 · what your app reads". */
function StepDots({ steps }: { steps: ReturnType<typeof stepStates<SetupStep>> }) {
  const at = steps.findIndex((x) => x.state === 'current')
  const now = steps[at]
  return now ? (
    <div className="iii-cloud-setup-dots">
      <span className="iii-cloud-setup-dotrow" aria-hidden>
        {steps.map((x) => (
          <span key={x.id} data-state={x.state} />
        ))}
      </span>
      <span className="iii-cloud-setup-dotname">{now.title}</span>
      <span className="iii-cloud-faint">
        step {at + 1} of {steps.length}
        {now.sub ? ` · ${now.sub}` : ''}
      </span>
    </div>
  ) : null
}

function Head({ title, lead }: { title: string; lead?: ReactNode }) {
  return (
    <div className="iii-cloud-setup-head">
      <h2 className="iii-cloud-setup-title">{title}</h2>
      {lead ? <p className="iii-cloud-setup-lead">{lead}</p> : null}
    </div>
  )
}

/** A label and its value (and an action at the end), one row of a facts panel. */
function Fact({ label, end, children }: { label: string; end?: ReactNode; children: ReactNode }) {
  return (
    <div className="iii-cloud-setup-fact">
      <span className="iii-cloud-setup-key">{label}</span>
      <span className="iii-cloud-setup-val">{children}</span>
      {end ? <span className="iii-cloud-setup-end">{end}</span> : null}
    </div>
  )
}

type Choice = { id: string; label: ReactNode; description?: ReactNode; trailing?: ReactNode; disabled?: boolean }

/** One choice of a few, as rows: the list walks with the arrows; Space or Enter picks. Selection stays neutral. */
function Choices({
  label,
  value,
  items,
  onPick,
}: {
  label: string
  value: string | undefined
  items: Choice[]
  onPick: (id: string) => void
}) {
  return (
    <List role="radiogroup" aria-label={label} className="iii-cloud-setup-choices">
      {items.map((c) => (
        <ListItem
          key={c.id}
          role="radio"
          aria-checked={c.id === value}
          // `data-selected`, not `selected`: a radio has no pressed state.
          data-selected={c.id === value ? '' : undefined}
          disabled={c.disabled}
          leading={c.id === value ? <CircleDot /> : <Circle />}
          label={c.label}
          description={c.description}
          trailing={c.trailing}
          onClick={() => onPick(c.id)}
        />
      ))}
    </List>
  )
}

/* ── 1. Start and sign in (Onb1) ──────────────────────────────────── */

const MOMENTS = [
  ['Approve the sign-in', 'once, in your browser'],
  ['Give the values your app reads', 'from your .env files, or typed'],
  ['Answer a few decisions', 'each with a recommended choice'],
  ['Enter secrets', 'the agent never sees them'],
  ['Confirm the deploy', 'nothing goes live before'],
] as const

function SignIn({ host, status, go, adding, actions }: StepProps) {
  const signedIn = !needsConnect(status) && !signInRequired(status)
  return (
    <>
      <StepHeader
        title="Put your project on iii cloud"
        sub="This session takes the code on this machine to a live address. Your agent does the fixing in a separate copy of your code, so your own files stay as they are. You step in five times."
        forward={
          signedIn ? (
            <Button variant="primary" onClick={() => go({ at: 'setup', ...(adding ? { adding: true } : {}) })}>
              Continue
            </Button>
          ) : null
        }
      />
      <ol className="iii-cloud-moments" aria-label="Where you step in">
        {MOMENTS.map(([title, sub], i) => (
          <li key={title}>
            <span className="iii-cloud-mono iii-cloud-faint">{String(i + 1).padStart(2, '0')}</span>
            <span className="iii-cloud-setup-val">
              <span className="iii-cloud-moment">{title}</span>
              <span className="iii-cloud-desc">{sub}</span>
            </span>
          </li>
        ))}
      </ol>
      {needsConnect(status) ? (
        <ConnectView host={host} connection={status.connection} />
      ) : signInRequired(status) ? (
        <Notice
          variant="warn"
          headline="Sign in again"
          detail="iii cloud no longer accepts your sign-in."
          action={
            <Button
              size="sm"
              variant="primary"
              disabled={actions.busy === 'connect'}
              onClick={() => actions.call('connect', 'iii-cloud::connect', { method: 'browser' })}
            >
              Sign in again
            </Button>
          }
        />
      ) : (
        <Notice variant="success" headline="Signed in" detail={whoLine(status)} />
      )}
    </>
  )
}

/* ── 2. Your code (Onb2) ──────────────────────────────────────────── */

/** The app files to pick from: the repository's on GitHub before it is imported, else the worktree's. */
const appFiles = (s: Status): ComposeFile[] => s.project?.importable?.manifests ?? s.compose_files ?? []

/** The picked app file, else the recommended one: the environment's, else one with a cloud section. */
function pickFile(s: Status, picked: string | null): ComposeFile | undefined {
  const ok = appFiles(s).filter((f) => f.choosable)
  const current = composeFile(s.project?.environment?.manifest)
  return (
    ok.find((f) => f.path === picked) ??
    ok.find((f) => f.path === current) ??
    ok.find((f) => f.has_cloud_block) ??
    ok[0]
  )
}

function fileNote(f: ComposeFile, recommended: boolean): string {
  return [
    recommended ? 'Recommended' : null,
    f.has_cloud_block === true ? 'has a cloud section' : f.has_cloud_block === false ? 'no cloud section' : null,
    f.workers != null ? plural(f.workers, 'worker') : null,
    f.suggested_environment ? `as ${f.suggested_environment}` : null,
    f.choosable ? null : (COMPOSE_ISSUE[f.issue ?? ''] ?? f.issue ?? "iii cloud can't take it"),
  ]
    .filter(Boolean)
    .join(' · ')
}

function Code({
  status,
  go,
  step,
  cancel,
  adding,
  selected,
  read,
  actions,
  draft,
  edit,
  workingDir,
}: StepProps & { workingDir?: string | null }) {
  const project = status.project
  const resolving = actions.busy === 'resolve'
  const [askCheckout, setAskCheckout] = useState(false)
  const resolve = (path: string, team?: string) => {
    // Stay on this step: once the folder resolves, the status points at the next one. Another folder while a
    // project is in use is added beside it (the worker keeps it as the candidate), so the step reads that one.
    if (adding || (selected?.workspace && path !== selected.path)) go({ at: 'setup', step: 'confirm', adding: true })
    else step('code')
    read(path, team)
  }
  // The chat's folder resolves once, on arrival (adding reads its own in SetupArea).
  useEffect(() => {
    if (!adding && !project && draft.path) resolve(draft.path)
  }, [])
  const files = appFiles(status)
  const recommended = pickFile(status, null)
  const chosen = pickFile(status, draft.manifest)
  const changed = project?.checkout.changed_files.length ?? 0
  const base = project?.environment?.branch ?? project?.default_branch ?? 'main'
  const ahead = project?.checkout.ahead ?? 0

  return (
    <>
      <StepHeader
        title="Which code goes to the cloud?"
        sub="Pick the folder of your project. The worker reads it and fills in the rest."
        // Nothing is called: adding goes back to Start, a project in use to its home.
        back={
          cancel ? (
            <StepLink onClick={() => go(cancel)}>Cancel</StepLink>
          ) : (
            <StepLink onClick={() => go({ at: 'setup', step: 'signin' })}>Back</StepLink>
          )
        }
        forward={
          <Button variant="primary" disabled={!project || resolving} onClick={() => step('target')}>
            Continue
          </Button>
        }
      />
      <div className="iii-cloud-setup-section">
        <span className="iii-cloud-setup-label">Project folder</span>
        <DirectoryPicker value={draft.path || null} defaultDir={workingDir ?? null} onChange={(dir) => resolve(dir)} />
      </div>
      {resolving ? (
        <Skeleton className="iii-cloud-setup-skel" />
      ) : project ? (
        <Panel aria-label="What the worker found">
          <PanelHeader className="iii-cloud-panel-head">Found in this folder</PanelHeader>
          <PanelBody className="iii-cloud-setup-rows">
            <Fact label="Repository">
              <span>
                <Mono>{project.repository}</Mono>
                <span className="iii-cloud-faint">
                  {' '}
                  · on GitHub{project.default_branch ? ` · default branch ${project.default_branch}` : ''}
                  {project.github_access === false ? " · iii cloud can't see it yet" : ''}
                </span>
              </span>
            </Fact>
            <Fact label="You are on">
              <span>
                <Mono>{project.checkout.branch ?? 'a detached HEAD'}</Mono>
                {ahead ? <span className="iii-cloud-faint"> · {plural(ahead, 'commit')} ahead of GitHub</span> : null}
              </span>
            </Fact>
            {!project.team && project.teams.length > 0 ? (
              <Fact label="Team">
                <Select
                  aria-label="Team"
                  placeholder="Choose a team"
                  value={undefined}
                  options={project.teams.map((t) => ({ value: t.id, label: t.name }))}
                  onChange={(id: string) => resolve(project.path, id)}
                />
              </Fact>
            ) : null}
            <Fact label="App file">
              {files.length ? (
                <Choices
                  label="App file the cloud runs"
                  value={chosen?.path}
                  items={files.map((f) => ({
                    id: f.path,
                    label: <Mono>{f.path}</Mono>,
                    description: fileNote(f, f.path === recommended?.path),
                    disabled: !f.choosable,
                  }))}
                  onPick={(manifest) => edit({ manifest })}
                />
              ) : project.environment ? (
                <Mono>{composeFile(project.environment.manifest)}</Mono>
              ) : (
                <span className="iii-cloud-faint">None listed yet</span>
              )}
            </Fact>
          </PanelBody>
        </Panel>
      ) : null}
      {project ? (
        <div className="iii-cloud-setup-note">
          <Info aria-hidden />
          <div className="iii-cloud-setup-val">
            {draft.mode === 'worktree' ? (
              <>
                <span>
                  Your agent will work in a separate copy of this repository, starting from GitHub's <Mono>{base}</Mono>
                  .{changed ? ` You have ${plural(changed, 'file')} changed here that aren't on GitHub.` : ''}
                </span>
                {changed ? (
                  <Checkbox
                    label={`Bring my ${changed} changed ${changed === 1 ? 'file' : 'files'} into the copy`}
                    checked={draft.bring}
                    onChange={(e) => edit({ bring: e.target.checked })}
                  />
                ) : null}
                <Button variant="ghost" size="sm" className="iii-cloud-disclose" onClick={() => setAskCheckout(true)}>
                  Let it edit this folder instead…
                </Button>
              </>
            ) : (
              <>
                <span>Your agent will change the files in this folder directly, with no separate copy.</span>
                <Button
                  variant="ghost"
                  size="sm"
                  className="iii-cloud-disclose"
                  onClick={() => edit({ mode: 'worktree' })}
                >
                  Use a separate copy instead
                </Button>
              </>
            )}
          </div>
        </div>
      ) : null}
      <ConfirmDialog
        open={askCheckout}
        onOpenChange={setAskCheckout}
        title="Let your agent edit your own files?"
        description="Your agent will change the files in your folder directly, with no separate copy. Anything you have not saved or committed could get mixed with its changes. A separate copy is safer."
        confirmLabel="Edit my own files"
        onConfirm={() => {
          edit({ mode: 'checkout' })
          setAskCheckout(false)
        }}
      />
    </>
  )
}

/* ── 3. Where it runs (Onb3) ──────────────────────────────────────── */

const NEW = '__new__'
const DELETED = ['PROJECT_DELETED', 'ENVIRONMENT_DELETED']

type CloudDeployment = { status: string; current?: boolean }

/** "Live", "Last deploy failed" or "Nothing live yet", from the environment's deployments in iii cloud. */
function EnvLive({ host, status, envId, readable }: { host: Host; status: Status; envId: string; readable: boolean }) {
  const { data } = useCloud<{ deployments?: CloudDeployment[] }>(host, status, envId, readable ? '/deployments' : null)
  if (!data) return null
  const list = data.deployments ?? []
  const newest = list[0]
  if (newest?.status === 'failed') return <StatusChip tone="danger">Last deploy failed</StatusChip>
  if (list.some((d) => d.current) || newest?.status === 'ready') return <StatusChip tone="success">Live</StatusChip>
  if (newest && ['queued', 'in_progress', 'needs_action'].includes(newest.status))
    return <StatusChip live>Deploying</StatusChip>
  return <StatusChip>Nothing live yet</StatusChip>
}

function Target({
  host,
  status,
  go,
  step,
  cancel,
  adding,
  actions,
  draft,
  edit,
  newProject: openNew,
}: StepProps & { newProject?: boolean }) {
  const project = status.project
  const checking = useChecking(host)
  const launching = useLaunch()?.kind === 'launching'
  const [localCps, setLocalCps] = useState<CloudProjectChoice[] | null>(null)
  const [newProject, setNewProject] = useState(!!openNew)
  const [newEnv, setNewEnv] = useState(false)
  const [showOthers, setShowOthers] = useState(false)
  if (!project) return null

  const raw: CloudProjectChoice[] =
    localCps ?? project.cloud_projects ?? (project.cloud_project ? [project.cloud_project] : [])
  const cloudProjects = raw
    .filter((c) => !c.deleted)
    .map((c) => ({ ...c, environments: c.environments?.filter((e) => !e.deleted) }))
  const creating = newProject || (cloudProjects.length === 0 && !project.cloud_project)
  const cp =
    cloudProjects.find((c) => c.id === draft.cp) ??
    cloudProjects.find((c) => c.id === project.cloud_project?.id) ??
    cloudProjects[0]
  const envList = cp?.environments ?? project.environments
  const env =
    envList.find((e) => e.id === draft.env) ??
    envList.find((e) => e.id === project.environment?.id) ??
    envList.find((e) => e.production) ??
    envList[0]
  const manifestOf = (e: typeof env) =>
    e && 'manifest' in e && e.manifest
      ? e.manifest
      : e?.id === project.environment?.id
        ? project.environment?.manifest
        : null
  const others = project.other_cloud_projects ?? []
  const severalTeams = new Set(cloudProjects.map((c) => c.team?.id)).size > 1
  // The source the new environment form starts on: a non-production one, so production's secrets stay there.
  const copyFrom = envList.find((e) => !e.production) ?? envList[0]
  const err = actions.error

  const pickProject = (id: string) => {
    const list = cloudProjects.find((c) => c.id === id)?.environments ?? []
    setNewProject(false)
    setNewEnv(false)
    edit({ cp: id, env: (list.find((e) => e.production) ?? list[0])?.id ?? null })
  }
  const onProjectCreated = (out: Created, envName: string) => {
    const list = out.cloud_projects ?? cloudProjects
    setLocalCps(list)
    setNewProject(false)
    // The ids come back from core: select them directly.
    const made =
      list.find((c) => c.id === out.project_id) ??
      list.find((c) => !cloudProjects.some((o) => o.id === c.id)) ??
      list[list.length - 1]
    if (made) {
      const e = out.environment_id ?? (made.environments?.find((x) => x.name === envName) ?? made.environments?.[0])?.id
      edit({ cp: made.id, env: e ?? null })
    }
  }
  const onEnvCreated = (out: Created, name: string) => {
    const list = out.cloud_projects ?? cloudProjects
    setLocalCps(list)
    setNewEnv(false)
    const pid = out.project_id ?? cp?.id ?? null
    const e = list.find((c) => c.id === pid)?.environments?.find((x) => x.name === name)
    edit({ cp: pid, env: out.environment_id ?? e?.id ?? draft.env })
  }
  const start = async () => {
    if (!env) return
    const out = await actions.call('use', 'iii-cloud::project::use', {
      path: project.path,
      ...(cp ? { cloud_project_id: cp.id } : {}),
      environment_id: env.id,
      mode: draft.mode,
      ...(draft.bring && draft.mode === 'worktree' ? { bring_changes: true } : {}),
    })
    if (out === null) return
    // Selected now (an added folder too, so add mode is over): the values the project in use reads, then its check.
    go({ at: 'setup', step: 'confirm' })
    step('values')
  }
  const file = pickFile(status, draft.manifest)
  const importChoice =
    (project.importable?.manifests.length ?? 0) > 1 && file
      ? { path: file.path, environment: file.suggested_environment }
      : undefined
  const block = creating ? 'Create the project first.' : newEnv ? 'Create the environment first.' : null

  return (
    <>
      <StepHeader
        title="Where should it run?"
        sub="A project holds your app on iii cloud. Each environment is one live copy of it, with its own address, database and secrets."
        back={cancel ? <StepLink onClick={() => go(cancel)}>Cancel</StepLink> : null}
        forward={
          <Button variant="primary" disabled={!env || !!block || actions.busy === 'use' || checking} onClick={start}>
            Continue
          </Button>
        }
        reason={block ?? 'Next: the values your app reads. Nothing is deployed.'}
      />
      {err ? (
        <ErrorPanel
          error={err}
          action={
            DELETED.includes(err.code) ? (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => {
                  actions.setError(null)
                  if (err.code === 'ENVIRONMENT_DELETED')
                    return edit({ env: envList.find((e) => e.id !== env?.id)?.id ?? null })
                  const next = cloudProjects.find((c) => c.id !== cp?.id) ?? cloudProjects[0]
                  if (next) pickProject(next.id)
                }}
              >
                Choose another
              </Button>
            ) : undefined
          }
        />
      ) : null}
      <section className="iii-cloud-setup-section" aria-label="Project">
        <span className="iii-cloud-setup-label">Project</span>
        <Panel>
          <Choices
            label="Project"
            value={creating ? NEW : cp?.id}
            items={[
              ...cloudProjects.map((c) => ({
                id: c.id,
                label: c.name,
                description: [
                  `Built from ${project.repository}`,
                  plural(c.environments?.length ?? project.environments.length, 'environment'),
                  severalTeams && c.team ? `team ${c.team.name}` : null,
                ]
                  .filter(Boolean)
                  .join(' · '),
                trailing: 'this repository',
              })),
              {
                id: NEW,
                label: 'New project',
                description: `Imports ${project.repository} and creates its first environment`,
              },
            ]}
            onPick={(id) => {
              if (id !== NEW) return pickProject(id)
              setNewProject(true)
              setNewEnv(false)
            }}
          />
          {others.length ? (
            <div className="iii-cloud-setup-others">
              <Button
                variant="ghost"
                size="sm"
                className="iii-cloud-disclose"
                aria-expanded={showOthers}
                onClick={() => setShowOthers(!showOthers)}
              >
                {showOthers ? <ChevronDown /> : <ChevronRight />}
                {plural(others.length, 'project')}
                {project.team ? ` in team ${project.team.name}` : ''}{' '}
                {others.length === 1 ? 'uses another repository' : 'use other repositories'}
              </Button>
              {showOthers ? (
                <p className="iii-cloud-desc">
                  {others.map((o) => o.name).join(', ')}. Open their folder to manage them.
                </p>
              ) : null}
            </div>
          ) : null}
        </Panel>
        {creating && project.github_access === false && !project.cloud_project ? (
          <Notice
            variant="info"
            headline={`iii cloud can't see ${project.repository} on GitHub yet`}
            detail="Your agent imports it; you authorize iii cloud on GitHub once."
            action={
              <Button
                size="sm"
                variant="secondary"
                disabled={!host.chat?.compose || launching}
                onClick={() =>
                  void handOff(host, status, 'import', undefined, importChoice, undefined, { candidate: adding })
                }
              >
                Ask the agent to import it
              </Button>
            }
          />
        ) : creating ? (
          <NewProjectForm
            host={host}
            status={status}
            path={project.path}
            onCreated={onProjectCreated}
            onCancel={cloudProjects.length > 0 ? () => setNewProject(false) : undefined}
          />
        ) : null}
      </section>
      {!creating && cp ? (
        <section className="iii-cloud-setup-section" aria-label="Environment">
          <span className="iii-cloud-setup-label">Environment in {cp.name}</span>
          <Panel>
            <Choices
              label="Environment"
              value={newEnv ? NEW : env?.id}
              items={[
                ...envList.map((e) => ({
                  id: e.id,
                  label: e.name,
                  description: (
                    <>
                      Follows <Mono>{e.branch}</Mono> ·{' '}
                      {isDirectEnv(status, e)
                        ? 'deploys after the check, from this machine'
                        : 'deploys when its pull request is merged'}
                    </>
                  ),
                  trailing: (
                    <EnvLive
                      host={host}
                      status={status}
                      envId={e.id}
                      readable={!cp.team || cp.team.id === project.team?.id}
                    />
                  ),
                })),
                {
                  id: NEW,
                  label: 'New environment',
                  description: `Pick a name and the branch it follows${copyFrom ? `; settings copy from ${copyFrom.name}` : ''}`,
                },
              ]}
              onPick={(id) => {
                setNewEnv(id === NEW)
                if (id !== NEW) edit({ cp: cp.id, env: id })
              }}
            />
          </Panel>
          {newEnv ? (
            <NewEnvironmentForm
              host={host}
              project={cp}
              defaultBranch={project.default_branch ?? env?.branch ?? 'main'}
              baseManifest={manifestOf(env)}
              copyFrom={envList.map((e) => ({ id: e.id, name: e.name }))}
              onCreated={onEnvCreated}
              onCancel={() => setNewEnv(false)}
              baseBranch={project.default_branch}
            />
          ) : null}
        </section>
      ) : null}
    </>
  )
}

/* ── 4. Values (Onb3b) ────────────────────────────────────────────── */

/** The names the app file reads, a value for each, then the check (with the app file picked in step 2). */
function ValuesStep({ host, status, go, step, actions, draft, narrow }: StepProps & { narrow: boolean }) {
  const checking = useChecking(host)
  const env = status.project?.environment
  const check = () => {
    // An app file picked in step 2 that isn't the environment's: the check plans that one.
    const picked = draft.manifest && draft.manifest !== composeFile(env?.manifest) ? { manifest: draft.manifest } : {}
    void actions.call('check', 'iii-cloud::check', { run: true, ...picked })
    step('check')
  }
  return (
    <ValuesEditor
      host={host}
      status={status}
      manifest={draft.manifest}
      mode="step"
      lead={
        narrow
          ? undefined
          : 'Pick where each value comes from. Nothing is saved until you press Save; secrets go only to iii cloud.'
      }
      narrow={narrow}
      disabled={checking || actions.busy === 'check'}
      onSaved={check}
      onSkip={check}
    />
  )
}

/* ── 5. Review and deploy (Onb6) ──────────────────────────────────── */

function Ship(props: StepProps) {
  const { status } = props
  const latest = status.deploys.find((d) => d.environment_id === status.project?.environment?.id) ?? null
  if (latest && ((latest.outcome === 'running' && !needsAction(latest)) || !status.check)) {
    return <Deploying {...props} deploy={latest} />
  }
  return <Review {...props} latest={latest} />
}

function Review({ host, status, actions, latest }: StepProps & { latest: Deploy | null }) {
  const launching = useLaunch()?.kind === 'launching'
  const project = status.project
  const env = project?.environment
  const check = status.check
  if (!project || !env || !check) return null
  const sha = sha7(check.commit)
  const direct = isDirectEnv(status, env)
  const pr = usablePr(status)
  const commits = check.commits ?? []
  const file = appFile(status)
  const summary = check.plan.summary
  const fixed = checklist(status)
    .rows.filter((r) => r.fixed && r.owner === 'agent')
    .map((r) => r.title)
  const { resources, overrides } = check.plan.confirmations
  const settings = overrides.map((o) => o.setting)
  const vars = status.variables.filter((v) => v.type !== 'builtin')
  const count = (s: string) => vars.filter((v) => v.status === s).length
  const ms = typicalDeployMs(status, env.id)
  const typical = ms === null ? null : minutesLabel(ms)
  const gh = `https://github.com/${project.repository}`
  const agent = status.agent && ['pull_request', 'deploy_fix'].includes(status.agent.kind ?? '') ? status.agent : null
  const held = latest && needsAction(latest) ? latest : null
  const failed = latest && isFailedDeploy(latest) ? latest : null
  const err = actions.error
  const stale = !!err && CHECK_AGAIN.has(err.code)
  const off = !check.ready || actions.busy === 'deploy' || stale
  const deploy = (payload: Record<string, unknown>) => void actions.call('deploy', 'iii-cloud::deploy', payload)
  const only = `${typical ? `About ${typical}. ` : ''}Only you can confirm this; your agent can't.`
  const fileNotes = [
    summary ? `${plural(summary.workers, 'worker')} in ${plural(summary.groups.length, 'group')}.` : null,
    fixed.length ? `Your agent fixed: ${fixed.join(', ')}.` : null,
    file.note ? `${file.note}.` : null,
  ]
    .filter(Boolean)
    .join(' ')

  // P1: the one forward action and its note, in the header; a pull request's link left of it.
  let action: ReactNode
  let note: string
  let link: ReactNode = null
  if (held) {
    action = (
      <Button variant="primary" disabled={off} onClick={() => deploy({ check_id: check.id, deployment_id: held.id })}>
        Confirm deploy
      </Button>
    )
    note = `iii cloud holds deploy #${held.number} until you approve what it creates. ${only}`
  } else if (direct) {
    action = (
      <Button variant="primary" disabled={off} onClick={() => deploy({ check_id: check.id })}>
        Deploy {sha} to {env.name}
      </Button>
    )
    note = only
  } else if (pr) {
    action = (
      <Button variant="primary" disabled={off} onClick={() => deploy({ check_id: check.id, pr: pr.number })}>
        Merge and deploy
      </Button>
    )
    link = <LinkButton href={pr.url}>Pull request #{pr.number}</LinkButton>
    note = `Merges pull request #${pr.number} into ${pr.base}. ${only}`
  } else {
    action = (
      <Button
        variant="primary"
        disabled={!check.ready || launching || !host.chat?.compose}
        onClick={() => void handOff(host, status, 'pull_request')}
      >
        {sendLabel(status, 'pull_request', 'Open the pull request')}
      </Button>
    )
    note = 'Your agent opens it with your GitHub account. Merging it starts the deploy, and this page follows it.'
  }

  return (
    <>
      <StepHeader
        title={`Review and deploy to ${env.name}`}
        sub={`Commit ${sha} passed the check on iii cloud and here. Nothing goes live before you confirm.`}
        back={link}
        forward={action}
        reason={note}
      />
      {err ? (
        <ErrorPanel
          error={err}
          action={
            stale ? (
              <Button
                size="sm"
                variant="primary"
                disabled={actions.busy === 'check'}
                onClick={() => {
                  actions.setError(null)
                  void actions.call('check', 'iii-cloud::check', withAppFile(status, { run: true }))
                }}
              >
                Check again
              </Button>
            ) : undefined
          }
        />
      ) : null}
      {failed ? <FailureCard host={host} status={status} deploy={failed} /> : null}
      <Panel aria-label="What will happen">
        <PanelBody className="iii-cloud-setup-rows">
          <Fact
            label="Your code"
            end={<LinkButton href={`${gh}/compare/${check.base}...${check.commit}`}>See changes</LinkButton>}
          >
            <span>
              {commits.length
                ? `${plural(commits.length, 'commit')} by your agent on top of ${env.branch}`
                : `Commit ${sha}`}
            </span>
            <span className="iii-cloud-desc">
              Goes to <Mono>{env.branch}</Mono>
              {direct
                ? `, the branch ${env.name} follows, when you confirm`
                : ' through a pull request; merging it starts the deploy'}
            </span>
          </Fact>
          <Fact
            label="App file"
            end={<LinkButton href={`${gh}/blob/${check.commit}/${file.path}`}>See the file</LinkButton>}
          >
            <Mono>{file.path}</Mono>
            {fileNotes ? <span className="iii-cloud-desc">{fileNotes}</span> : null}
          </Fact>
          {overrides.length ? (
            <Fact label="Environment" end={<StatusChip tone="warning">changes a setting</StatusChip>}>
              <span>Changes {plural(overrides.length, 'setting')} the console had</span>
              <span className="iii-cloud-desc">
                {settings.join(', ')} now {settings.length === 1 ? 'comes' : 'come'} from {file.path}.
              </span>
            </Fact>
          ) : null}
          {resources.length ? (
            <Fact label="Creates">
              {resources.map((r) => (
                <span key={`${r.kind}:${r.alias}`}>
                  A {r.tier ? `${r.tier} ` : ''}
                  {RESOURCE_KIND[r.kind] ?? r.kind} for <Mono>{r.binding ?? r.alias}</Mono>
                </span>
              ))}
            </Fact>
          ) : null}
          <Fact label="Secrets">
            {vars.length
              ? [
                  `${count('set')} set`,
                  count('empty_default') ? `${count('empty_default')} optional not set` : null,
                  count('missing') ? `${count('missing')} missing` : null,
                ]
                  .filter(Boolean)
                  .join(' · ')
              : 'None'}
          </Fact>
          {env.url ? (
            <Fact label="Address">
              <Mono>{env.url}</Mono>
            </Fact>
          ) : null}
        </PanelBody>
      </Panel>
      {agent ? <AgentRow host={host} agent={agent} branch={status.project?.workspace?.branch} /> : null}
    </>
  )
}

/** Deploying (Onb6): the stages as they go, the live log, and the person's way to stop it. */
function Deploying({ host, status, deploy: d }: StepProps & { deploy: Deploy }) {
  const now = useNow()
  const [ask, setAsk] = useState(false)
  const [error, setError] = useState<WorkerError | null>(null)
  const tail = d.log_tail.length > 0
  // The deploy carries no lines while it runs: read the release log every 5 s.
  const logs = useCloud<{ lines?: { message: string }[] }>(
    host,
    status,
    d.environment_id,
    tail ? null : `/logs?deployment_id=${d.id}&source=release`,
    { pollMs: 5000 },
  )
  const lines = tail ? d.log_tail : (logs.data?.lines ?? []).map((l) => l.message).reverse()
  const rows = deployRows(d)
  const current = rows.find((r) => r.tone === 'live')
  const envName =
    status.project?.environments.find((e) => e.id === d.environment_id)?.name ?? status.project?.environment?.name
  // The dialog stays open and busy until this settles, then closes itself: one cancel per confirm.
  const cancel = async () => {
    setError(null)
    try {
      await cloudWrite(host, status, d.environment_id, 'POST', `/deployments/${d.id}/cancel`)
      void hubOf(host).refresh(true)
    } catch (err) {
      const e = err as Partial<WorkerError>
      setError({ code: e.code ?? 'ERROR', message: e.message ?? '' })
    }
  }
  return (
    <>
      <div className="iii-cloud-setup-headrow">
        <Head
          title={`Deploying ${sha7(d.commit.sha)} to ${envName ?? 'the environment'}`}
          lead={`Deploy #${d.number} · started ${clock(d.created_at)} · ${minutesLabel(now - Date.parse(d.created_at))} so far`}
        />
        {current ? (
          <span aria-live="polite">
            <StatusChip live>{current.label}</StatusChip>
          </span>
        ) : null}
      </div>
      <Panel aria-label="Deploy stages">
        <ol className="iii-cloud-setup-stages">
          {rows.map((r) => (
            <li key={r.key} data-live={r.tone === 'live' ? '' : undefined}>
              <Dot tone={r.tone} />
              <span>
                {r.label}
                {r.detail ? <span className="iii-cloud-faint"> · {r.detail}</span> : null}
              </span>
              <span className="iii-cloud-setup-time">{r.time}</span>
            </li>
          ))}
        </ol>
        <div className="iii-cloud-setup-log">
          <div className="iii-cloud-row">
            <span className="iii-cloud-lbl iii-cloud-grow">Live log</span>
            <LinkButton href={d.console_url}>Open full log</LinkButton>
          </div>
          {lines.length ? (
            <TerminalStream label={tail ? 'deploy' : 'release'} text={lines.join('\n')} />
          ) : (
            <span className="iii-cloud-desc">No log lines yet.</span>
          )}
        </div>
      </Panel>
      {error ? <ErrorPanel error={error} /> : null}
      <div className="iii-cloud-row">
        <Button variant="secondary" size="sm" onClick={() => setAsk(true)}>
          Cancel the deploy
        </Button>
      </div>
      <ConfirmDialog
        open={ask}
        onOpenChange={setAsk}
        title={`Cancel deploy #${d.number}?`}
        description="iii cloud stops it at the stage it is in. Nothing is deleted."
        confirmLabel={`Cancel deploy #${d.number}`}
        cancelLabel="Keep deploying"
        tone="danger"
        onConfirm={cancel}
      />
    </>
  )
}

/* ── 6. Live (Onb7) ───────────────────────────────────────────────── */

type Health = { status: 'healthy' | 'degraded' | 'down' | 'stopped'; workers: { ready: boolean }[] }
const HEALTH_TONE = { healthy: 'ok', degraded: 'warn', down: 'alert', stopped: 'edge' } as const

function Live({ host, status, go }: StepProps) {
  const project = status.project
  const env = project?.environment
  const health = useCloud<Health>(host, status, env?.id ?? '', env ? '/health' : null)
  const url = env?.url ? (/^https?:\/\//.test(env.url) ? env.url : `https://${env.url}`) : ''
  const copy = useCopyFlash(url)
  if (!project || !env) return null
  const d = servingDeploy(status, env.id)
  const name = project.cloud_project?.name ?? project.repository
  // Newest first: the last one kept is the first check of this environment.
  const first = (status.checks ?? []).filter((c) => c.environment_id === env.id).at(-1)
  let story = ''
  if (d && first) {
    const since = Date.parse(first.checked_at)
    const failed = status.deploys.filter(
      (x) => x.environment_id === env.id && isFailedDeploy(x) && Date.parse(x.created_at) >= since,
    ).length
    story = ` It took ${minutesLabel(Date.parse(d.finished_at ?? d.created_at) - since)} from the first check${failed ? `, with ${plural(failed, 'failed deploy')} along the way` : ''}.`
  }
  const h = health.data
  const shipped = !!d && d.commit.sha === status.check?.commit

  return (
    <>
      <div className="iii-cloud-setup-head">
        <span className="iii-cloud-setup-done" aria-hidden>
          <Check />
        </span>
        <h2 className="iii-cloud-setup-title">
          {name} is live on {env.name}
        </h2>
        {d ? (
          <p className="iii-cloud-setup-lead">
            Deploy #{d.number} is serving commit <Mono>{sha7(d.commit.sha)}</Mono>.{story}
          </p>
        ) : null}
      </div>
      <Panel aria-label="Live checks">
        {url ? (
          <div className="iii-cloud-setup-url">
            <span className="iii-cloud-mono iii-cloud-grow">{url}</span>
            <LinkButton href={url}>Open</LinkButton>
            <Button variant="secondary" size="sm" onClick={copy.copy}>
              {copy.state === 'copied' ? 'Copied' : 'Copy'}
            </Button>
          </div>
        ) : null}
        <ul className="iii-cloud-setup-checks">
          <li>
            {h ? (
              <>
                <Dot tone={HEALTH_TONE[h.status] ?? 'edge'} />
                <span className="iii-cloud-grow">
                  {h.status === 'healthy' ? "iii cloud says it's healthy" : `iii cloud says it's ${h.status}`}
                </span>
                <span className="iii-cloud-desc">
                  {h.workers.filter((w) => w.ready).length} of {h.workers.length} workers up
                </span>
              </>
            ) : health.error ? (
              <>
                <Dot tone="ghost" />
                <span className="iii-cloud-grow">{health.error.message}</span>
              </>
            ) : (
              <Skeleton className="iii-cloud-setup-skelrow" />
            )}
          </li>
          {d ? (
            <li>
              <Dot tone={shipped ? 'ok' : 'warn'} />
              <span className="iii-cloud-grow">
                {shipped ? 'Serving the commit you shipped' : 'Serving a commit other than the one you checked'}
              </span>
              <span className="iii-cloud-mono iii-cloud-desc">version {sha7(d.commit.sha)}</span>
            </li>
          ) : null}
        </ul>
      </Panel>
      <div className="iii-cloud-setup-after">
        <span className="iii-cloud-moment">From now on</span>
        <span>
          Every deploy from this machine goes through the same check, with your agent fixing what it finds. This page
          becomes the place to watch deploys, read failures, change secrets and settings, and add environments.
        </span>
      </div>
      <div className="iii-cloud-row">
        <Button variant="primary" onClick={() => go({ at: 'env', env: '' })}>
          Go to {name}
        </Button>
      </div>
    </>
  )
}

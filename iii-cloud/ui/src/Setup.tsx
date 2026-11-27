// First time (First1 to First2c): sign in once, then one screen that says what the page found (folder, project,
// environment, compose file), each row changeable in place. The button creates what is new, selects it, starts
// the check and goes to the Deploy place. Values, the check, the review and "live" belong to that place.

import type { Host } from '@iii-dev/console-ui'
import { Button, ConfirmDialog, DirectoryPicker, List, ListItem, PageMain, Select, Skeleton } from '@iii-dev/console-ui'
import { Circle, CircleDot } from 'lucide-react'
import { type ReactNode, useEffect, useState } from 'react'
import { useCloud } from './api'
import { useLaunch } from './asked'
import { ConnectView } from './ConnectView'
import {
  type Created,
  CreateError,
  createEnvironment,
  createProject,
  type NewEnv,
  NewEnvironmentFields,
  type NewProject,
  NewProjectFields,
  newEnvReady,
} from './CreateForms'
import { handOff } from './handoff'
import { hubOf, useActions, useChecking } from './hub'
import {
  COMPOSE_ISSUE,
  composeFile,
  isDirectEnv,
  liveOf,
  needsConnect,
  plural,
  sha7,
  signInRequired,
  tilde,
} from './model'
import { DetailRow, ErrorPanel, Mono, Notice, PathBar, StepLink } from './parts'
import { type AreaProps, pathOf, type Route, type RouteAt } from './route'
import type { CloudProjectChoice, ComposeFile, DeploymentList, Project, Status } from './types'

/** The compose files to pick from: the repository's on GitHub before it is imported, else the worktree's. */
const appFiles = (s: Status): ComposeFile[] => s.project?.importable?.manifests ?? s.compose_files ?? []

/** The picked compose file, else the recommended one: the environment's, else one with a cloud section. */
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

function fileNote(f: ComposeFile, recommended: boolean, suggest = true): string {
  return [
    recommended ? 'Recommended' : null,
    f.has_cloud_block === true ? 'It has a cloud section' : f.has_cloud_block === false ? 'No cloud section' : null,
    f.workers != null ? plural(f.workers, 'worker') : null,
    suggest && f.suggested_environment ? `as ${f.suggested_environment}` : null,
    f.choosable ? null : (COMPOSE_ISSUE[f.issue ?? ''] ?? f.issue ?? "iii cloud can't take it"),
  ]
    .filter(Boolean)
    .join(' · ')
}

const NEW = '__new__'
const DELETED = ['PROJECT_DELETED', 'ENVIRONMENT_DELETED']
type Row = 'folder' | 'project' | 'env' | 'file'

/** First time, from the route: sign in until the person is signed in, then confirm. */
export function SetupArea(props: AreaProps<RouteAt<'setup'>>) {
  const { host, status, route, go, narrow } = props
  const signedOut = needsConnect(status) || signInRequired(status)
  // Leaving the sign-in goes back to Projects once there is a project to go back to.
  const cancel: Route | null =
    route.adding || status.project?.workspace || status.recent_projects?.length ? { at: 'start' } : null
  return (
    <PageMain className="iii-cloud-main">
      <PathBar path={pathOf(route, status)} host={host} status={status} route={route} go={go} narrow={narrow} rule />
      <div className="iii-cloud-setup" data-step={signedOut ? 'signin' : 'confirm'}>
        {signedOut ? (
          <SignIn host={host} status={status} cancel={cancel} go={go} />
        ) : (
          <Confirm {...props} cancel={cancel} />
        )}
      </div>
    </PageMain>
  )
}

function SignIn({
  host,
  status,
  cancel,
  go,
}: {
  host: Host
  status: Status
  cancel: Route | null
  go: (r: Route) => void
}) {
  const actions = useActions(host)
  if (needsConnect(status))
    return <ConnectView host={host} connection={status.connection} onCancel={cancel ? () => go(cancel) : undefined} />
  return (
    <>
      <h2 className="iii-cloud-setup-title">Sign in to iii cloud</h2>
      <Notice
        variant="warn"
        headline="Your sign-in expired"
        detail="iii cloud no longer accepts it. Sign in again, then this page goes on by itself."
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
    </>
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

/** "Live · 9c04e7f", "Failed" or "Not deployed yet": what iii cloud has in an environment. */
function EnvLive({ host, status, envId, readable }: { host: Host; status: Status; envId: string; readable: boolean }) {
  const { data } = useCloud<DeploymentList>(host, status, envId, readable ? '/deployments' : null)
  const live = liveOf(data?.deployments)
  if (!live) return null
  const word =
    live.label === 'Nothing live yet'
      ? 'Not deployed yet'
      : live.label === 'Last deployment failed'
        ? 'Failed'
        : live.label === 'Live' && live.sha
          ? `Live · ${sha7(live.sha)}`
          : live.label
  return <span className="iii-cloud-desc">{word}</span>
}

/** A row that is open: its label, what it is for, the editor, and Done. */
function Open({
  label,
  help,
  children,
  onDone,
}: {
  label: string
  help?: string
  children: ReactNode
  onDone: () => void
}) {
  return (
    <div className="iii-cloud-detail-open">
      <div className="iii-cloud-detail-openhead">
        <span className="iii-cloud-setup-label">{label}</span>
        {help ? <span className="iii-cloud-desc">{help}</span> : null}
      </div>
      {children}
      <div className="iii-cloud-row">
        <Button variant="secondary" size="sm" onClick={onDone}>
          Done
        </Button>
        <span className="iii-cloud-desc">Picking one changes nothing in iii cloud. Only the button below does.</span>
      </div>
    </div>
  )
}

function Confirm({
  host,
  status,
  route,
  go,
  workingDir,
  cancel,
}: AreaProps<RouteAt<'setup'>> & { cancel: Route | null }) {
  const actions = useActions(host)
  const checking = useChecking(host)
  const launching = useLaunch()?.kind === 'launching'
  const adding = !!route.adding
  const selected = status.project
  // Adding leaves the project in use selected: the screen reads the folder `resolve` read beside it (the
  // candidate), once this screen read it with success, and only for the folder picked.
  const [fresh, setFresh] = useState(false)
  const [path, setPath] = useState(
    (adding
      ? (status.candidate?.path ?? workingDir ?? (route.newProject ? selected?.path : null))
      : (selected?.path ?? workingDir)) ?? '',
  )
  const candidate = fresh && status.candidate?.path === path ? status.candidate : null
  const view: Status = adding ? { ...status, project: candidate, check: null, compose_files: [] } : status
  const project = view.project

  const [open, setOpen] = useState<Row | null>(null)
  const [mode, setMode] = useState<'worktree' | 'checkout'>((adding ? null : selected?.workspace?.mode) ?? 'worktree')
  const [askCheckout, setAskCheckout] = useState(false)
  const [cpPick, setCpPick] = useState<string | null>(route.newProject ? NEW : null)
  const [envPick, setEnvPick] = useState<string | null>(null)
  const [manifest, setManifest] = useState<string | null>(null)
  const [localCps, setLocalCps] = useState<CloudProjectChoice[] | null>(null)
  const [np, setNp] = useState<NewProject>({ team: '', env: 'test', production: false })
  const [ne, setNe] = useState<NewEnv>({ name: '', branch: '', copy: '' })

  const resolving = actions.busy === 'resolve'
  const read = (dir: string, team?: string) => {
    setPath(dir)
    setManifest(null)
    setCpPick(null)
    setEnvPick(null)
    setLocalCps(null)
    setFresh(false)
    void actions
      .call<Project>('resolve', 'iii-cloud::project::resolve', { path: dir, ...(team ? { team_id: team } : {}) })
      .then(async (out) => {
        if (out === null) return
        // The worker reads the repository's top folder; the candidate arrives with the status (no event).
        if (out?.path) setPath(out.path)
        await hubOf(host).refresh(false)
        setFresh(true)
      })
  }
  // The folder resolves once, on arrival: the chat's folder; adding reads its own (a new cloud project may be
  // for the folder in use).
  useEffect(() => {
    if (path && (adding || !selected)) read(path)
  }, [])
  const pickFolder = (dir: string) => {
    // Another folder while a project is in use is added beside it: the worker keeps it as the candidate.
    if (!adding && selected?.workspace && dir !== selected.path) go({ at: 'setup', step: 'confirm', adding: true })
    read(dir)
  }

  const teams = status.connection.teams
  const raw: CloudProjectChoice[] =
    localCps ?? project?.cloud_projects ?? (project?.cloud_project ? [project.cloud_project] : [])
  const cloudProjects = raw
    .filter((c) => !c.deleted)
    .map((c) => ({ ...c, environments: c.environments?.filter((e) => !e.deleted) }))
  const creatingProject = cpPick === NEW || (!!project && cloudProjects.length === 0 && !project.cloud_project)
  const cp = creatingProject
    ? undefined
    : (cloudProjects.find((c) => c.id === cpPick) ??
      cloudProjects.find((c) => c.id === project?.cloud_project?.id) ??
      cloudProjects[0])
  const envList = cp?.environments ?? project?.environments ?? []
  const creatingEnv = !creatingProject && (envPick === NEW || (!!cp && envList.length === 0))
  const env =
    creatingProject || creatingEnv
      ? undefined
      : (envList.find((e) => e.id === envPick) ??
        // The environment in use counts only once the project is in use; before, the one that isn't production.
        (project?.workspace ? envList.find((e) => e.id === project.environment?.id) : undefined) ??
        envList.find((e) => !e.production) ??
        envList.find((e) => e.production) ??
        envList[0])
  const file = pickFile(view, manifest)
  const files = appFiles(view)
  const repoName = project?.repository.split('/').pop() ?? ''
  const team = np.team || project?.team?.id || teams[0]?.id || ''
  const envName = creatingProject ? np.env.trim() : creatingEnv ? ne.name.trim() : (env?.name ?? '')
  const base = project?.default_branch ?? 'main'
  const newEnv: NewEnv = { ...ne, branch: ne.branch || base }
  const changed = project?.checkout.changed_files.length ?? 0
  const manifestOf = (e: typeof env) =>
    e && 'manifest' in e && e.manifest
      ? e.manifest
      : e?.id === project?.environment?.id
        ? project?.environment?.manifest
        : null
  const importChoice =
    (project?.importable?.manifests.length ?? 0) > 1 && file
      ? { path: file.path, environment: file.suggested_environment }
      : undefined
  const noAccess = creatingProject && project?.github_access === false && !project.cloud_project
  const err = actions.error

  const pickProject = (id: string) => {
    const list = cloudProjects.find((c) => c.id === id)?.environments ?? []
    setCpPick(id)
    setEnvPick((list.find((e) => !e.production) ?? list.find((e) => e.production) ?? list[0])?.id ?? null)
  }

  // Why the button waits, in words; null when it can go.
  const why = !project
    ? 'Pick the folder of your project first.'
    : !project.team && project.teams.length > 0
      ? 'Choose a team first.'
      : noAccess
        ? `iii cloud can't see ${project.repository} on GitHub yet.`
        : creatingProject && (!team || !np.env.trim())
          ? 'Name the first environment first.'
          : creatingEnv && !newEnvReady(newEnv)
            ? 'Name the new environment first.'
            : !creatingProject && !creatingEnv && !env
              ? 'Pick an environment first.'
              : null
  const busy = !!actions.busy || checking

  const go2 = async () => {
    if (!project || why) return
    let cpId = cp?.id
    let envId = env?.id
    if (creatingProject) {
      const out = await createProject(actions.call, project.path, file?.path ?? 'worker-compose.yaml', {
        ...np,
        team,
      })
      if (out === null) return
      const list = out.cloud_projects ?? cloudProjects
      const made =
        list.find((c) => c.id === out.project_id) ??
        list.find((c) => !cloudProjects.some((o) => o.id === c.id)) ??
        list[list.length - 1]
      if (!made) return
      cpId = made.id
      envId =
        out.environment_id ?? (made.environments?.find((x) => x.name === np.env.trim()) ?? made.environments?.[0])?.id
      setLocalCps(list)
      setCpPick(made.id)
      setEnvPick(envId ?? null)
    } else if (creatingEnv && cp) {
      const out: Created | null = await createEnvironment(
        actions.call,
        cp,
        envList.map((e) => ({ id: e.id, name: e.name })),
        file?.path ?? manifestOf(envList[0]),
        newEnv,
      )
      if (out === null) return
      const list = out.cloud_projects ?? cloudProjects
      const pid = out.project_id ?? cp.id
      envId =
        out.environment_id ??
        list.find((c) => c.id === pid)?.environments?.find((x) => x.name === newEnv.name.trim())?.id
      setLocalCps(list)
      setCpPick(pid)
      setEnvPick(envId ?? null)
    }
    if (!envId) return
    const used = await actions.call(
      'use',
      'iii-cloud::project::use',
      {
        path: project.path,
        ...(cpId ? { cloud_project_id: cpId } : {}),
        environment_id: envId,
        mode,
        // The copy the agent works in starts with the files changed here that aren't on GitHub.
        ...(mode === 'worktree' && changed ? { bring_changes: true } : {}),
      },
      { changesSource: true },
    )
    if (used === null) return
    // A compose file picked here that isn't the environment's: the check plans that one.
    const picked =
      manifest && manifest !== composeFile(manifestOf(env)) && !creatingProject && !creatingEnv ? { manifest } : {}
    // ...and a retry after a refusal checks what this Confirm picked, always: an older pick (a refused Change) never
    // outlives it.
    hubOf(host).setPickedManifest(envId, manifest || null)
    // The Deploy screen follows the check and shows a refusal: the local run can take minutes, so nothing waits here.
    void actions.call('check', 'iii-cloud::check', { run: true, ...picked })
    go({ at: 'deploy', env: envId })
  }

  // ── the four rows, closed ──
  const direct = (e: { id: string; branch: string }) =>
    isDirectEnv(status, e)
      ? 'You deploy it from this machine, after the check.'
      : `Deploys when a pull request into ${e.branch} is merged.`
  const projectValue = creatingProject ? `New project: ${repoName}` : (cp?.name ?? '')
  const projectNote = creatingProject
    ? `No project in ${teams.find((t) => t.id === team)?.name ? `team ${teams.find((t) => t.id === team)?.name}` : 'your team'} is built from this repository yet`
    : `It is built from this repository · ${plural(envList.length, 'environment')}`
  const envValue = creatingProject || creatingEnv ? `New environment: ${envName || '…'}` : (env?.name ?? 'None yet')
  const envNote = creatingProject
    ? np.production
      ? `Deploys when a pull request into ${base} is merged.`
      : 'You deploy it from this machine, after the check.'
    : creatingEnv
      ? `Deploys from ${newEnv.branch}. ${newEnv.copy === 'start-empty' ? 'You set everything.' : 'Its secrets and settings start as a copy.'}`
      : env
        ? direct(env)
        : undefined
  const fileValue = file?.path ?? composeFile(project?.environment?.manifest)
  const fileNoteText = file ? fileNote(file, false, false) : 'None found in this folder yet'
  const done = () => setOpen(null)
  const toggle = (r: Row) => () => setOpen(r)

  const folderEditor = (
    <Open label="Folder" onDone={done}>
      <DirectoryPicker value={path || null} defaultDir={workingDir ?? null} onChange={pickFolder} />
      {project ? (
        <>
          <dl className="iii-cloud-facts">
            <div className="iii-cloud-fact">
              <dt>Repository</dt>
              <dd>{project.repository} on GitHub</dd>
            </div>
            <div className="iii-cloud-fact">
              <dt>You are on</dt>
              <dd>
                {project.checkout.branch ?? 'a detached HEAD'}
                {project.checkout.ahead ? ` · ${plural(project.checkout.ahead, 'commit')} ahead of ${base}` : ''}
              </dd>
            </div>
            {changed ? (
              <div className="iii-cloud-fact">
                <dt>Not on GitHub</dt>
                <dd>{plural(changed, 'changed file')}; they go into your agent's copy</dd>
              </div>
            ) : null}
          </dl>
          <p className="iii-cloud-desc">
            Another folder reads again: its repository decides the project, the environment and the compose file below.
          </p>
          {mode === 'worktree' ? (
            <StepLink onClick={() => setAskCheckout(true)}>Let your agent edit this folder instead of a copy</StepLink>
          ) : (
            <>
              <p className="iii-cloud-desc">
                Your agent will change the files in this folder directly, with no separate copy.
              </p>
              <StepLink onClick={() => setMode('worktree')}>Use a separate copy instead</StepLink>
            </>
          )}
        </>
      ) : null}
    </Open>
  )

  const others = project?.other_cloud_projects ?? []
  const projectEditor = project ? (
    <Open label="Project" onDone={done}>
      <Choices
        label="Project"
        value={creatingProject ? NEW : cp?.id}
        items={[
          ...cloudProjects.map((c) => ({
            id: c.id,
            label: c.name,
            description: [
              `Built from ${project.repository}`,
              plural(c.environments?.length ?? project.environments.length, 'environment'),
              new Set(cloudProjects.map((x) => x.team?.id)).size > 1 && c.team ? `team ${c.team.name}` : null,
            ]
              .filter(Boolean)
              .join(' · '),
          })),
          {
            id: NEW,
            label: 'New project',
            description: `Imports ${project.repository} and creates its first environment`,
          },
        ]}
        onPick={(id) => (id === NEW ? setCpPick(NEW) : pickProject(id))}
      />
      {creatingProject ? <NewProjectFields part="team" teams={teams} value={{ ...np, team }} onChange={setNp} /> : null}
      {others.length ? (
        <p className="iii-cloud-desc">
          {plural(others.length, 'other project')}
          {project.team ? ` in team ${project.team.name}` : ''} {others.length === 1 ? 'is' : 'are'} built from{' '}
          {others.length === 1 ? 'another repository' : 'other repositories'}: {others.map((o) => o.name).join(', ')}.
          Open their folder to work on {others.length === 1 ? 'it' : 'them'}.
        </p>
      ) : null}
    </Open>
  ) : null

  const envEditor = project ? (
    <Open
      label="Environment"
      help="Each environment is one live copy of the app, with its own address, database and secrets."
      onDone={done}
    >
      {creatingProject ? (
        <NewProjectFields part="env" teams={teams} value={{ ...np, team }} onChange={setNp} />
      ) : (
        <>
          <Choices
            label="Environment"
            value={creatingEnv ? NEW : env?.id}
            items={[
              ...envList.map((e) => ({
                id: e.id,
                label: e.name,
                description: direct(e),
                trailing: (
                  <EnvLive
                    host={host}
                    status={status}
                    envId={e.id}
                    readable={!cp?.team || cp.team.id === project.team?.id}
                  />
                ),
              })),
              {
                id: NEW,
                label: 'New environment',
                description:
                  'You pick a name. Its secrets and settings start as a copy, and you deploy it from this machine.',
              },
            ]}
            onPick={setEnvPick}
          />
          {creatingEnv && cp ? (
            <NewEnvironmentFields
              project={cp}
              copyFrom={envList.map((e) => ({ id: e.id, name: e.name }))}
              baseBranch={project.default_branch}
              value={newEnv}
              onChange={setNe}
            />
          ) : null}
        </>
      )}
    </Open>
  ) : null

  const fileEditor = (
    <Open label="Compose file" onDone={done}>
      <Choices
        label="Compose file the cloud runs"
        value={file?.path}
        items={files.map((f) => ({
          id: f.path,
          label: <Mono>{f.path}</Mono>,
          description: fileNote(f, f.path === pickFile(view, null)?.path),
          disabled: !f.choosable,
        }))}
        onPick={setManifest}
      />
      <p className="iii-cloud-desc">
        The check runs with the file you pick.
        {env ? ` ${env.name} switches to it only when you confirm a deploy that passed with it.` : ''}
      </p>
    </Open>
  )

  const button = creatingProject
    ? `Create ${repoName} and check it`
    : creatingEnv
      ? `Create ${envName || 'the environment'} and check it`
      : 'Continue to the check'
  const nothing = 'Nothing is deployed until you confirm a deploy.'
  const effect = creatingProject
    ? `This creates the project and its ${envName} environment in iii cloud and starts the check. ${nothing}`
    : creatingEnv
      ? `This creates the ${envName} environment in iii cloud and starts the check. ${nothing}`
      : `This starts the check of ${env?.name ?? 'the environment'}. ${nothing}`
  const agent =
    mode === 'checkout'
      ? 'Your agent changes the files in this folder directly, with no separate copy.'
      : `Your agent works in a separate copy of the repository, so your folder stays as it is.${changed ? ` The ${plural(changed, 'file')} you changed on this machine that aren't on GitHub go into the copy too.` : ''}`
  const loading = resolving || (!project && !!path && !err)

  return (
    <>
      <div className="iii-cloud-setup-head">
        <h2 className="iii-cloud-setup-title">
          {project ? `Put ${repoName} on iii cloud` : 'Put this project on iii cloud'}
        </h2>
        <p className="iii-cloud-setup-lead">
          {project
            ? "The page read your folder and filled this in. Change anything that's wrong, then continue."
            : 'Pick the folder of your project. The page reads it and fills in the rest.'}
        </p>
      </div>
      {loading ? (
        <Skeleton className="iii-cloud-setup-skel" />
      ) : !project ? (
        folderEditor
      ) : (
        <div className="iii-cloud-detailcard">
          {open === 'folder' ? (
            folderEditor
          ) : (
            <DetailRow
              label="Folder"
              onChange={toggle('folder')}
              note={`${project.repository} on GitHub · you are on ${project.checkout.branch ?? 'a detached HEAD'}${project.checkout.ahead ? ` · ${plural(project.checkout.ahead, 'commit')} ahead of ${base}` : ''}${project.github_access === false ? " · iii cloud can't see it yet" : ''}`}
            >
              <Mono>{tilde(project.path)}</Mono>
            </DetailRow>
          )}
          {open === 'project' ? (
            projectEditor
          ) : (
            <DetailRow label="Project" onChange={toggle('project')} note={projectNote}>
              <strong>{projectValue}</strong>
            </DetailRow>
          )}
          {open === 'env' ? (
            envEditor
          ) : (
            <DetailRow label="Environment" onChange={toggle('env')} note={envNote}>
              <strong>{envValue}</strong>
            </DetailRow>
          )}
          {open === 'file' ? (
            fileEditor
          ) : (
            <DetailRow label="Compose file" onChange={toggle('file')} note={fileNoteText}>
              <Mono>{fileValue}</Mono>
            </DetailRow>
          )}
        </div>
      )}
      {project ? <p className="iii-cloud-desc">{agent}</p> : null}
      {project && !project.team && project.teams.length > 0 ? (
        <div className="iii-cloud-row">
          <span className="iii-cloud-setup-label">Team</span>
          <Select
            aria-label="Team"
            placeholder="Choose a team"
            value={undefined}
            options={project.teams.map((t) => ({ value: t.id, label: t.name }))}
            onChange={(id: string) => read(project.path, id)}
          />
        </div>
      ) : null}
      {noAccess && project ? (
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
      ) : null}
      {err ? (
        err.code === 'TEAM_FORBIDDEN' ? (
          <CreateError error={err} />
        ) : (
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
                      return setEnvPick(envList.find((e) => e.id !== env?.id)?.id ?? null)
                    const next = cloudProjects.find((c) => c.id !== cp?.id) ?? cloudProjects[0]
                    if (next) pickProject(next.id)
                  }}
                >
                  Choose another
                </Button>
              ) : undefined
            }
          />
        )
      ) : null}
      {project || cancel ? (
        <div className="iii-cloud-row">
          {project ? (
            <Button variant="primary" disabled={!!why || busy} onClick={go2}>
              {button}
            </Button>
          ) : null}
          {cancel ? <StepLink onClick={() => go(cancel)}>Cancel</StepLink> : null}
        </div>
      ) : null}
      {project ? <p className="iii-cloud-desc">{why ?? effect}</p> : null}
      <ConfirmDialog
        open={askCheckout}
        onOpenChange={setAskCheckout}
        title="Let your agent edit your own files?"
        description="Your agent will change the files in your folder directly, with no separate copy. Anything you have not saved or committed could get mixed with its changes. A separate copy is safer."
        confirmLabel="Edit my own files"
        onConfirm={() => {
          setMode('checkout')
          setAskCheckout(false)
        }}
      />
    </>
  )
}

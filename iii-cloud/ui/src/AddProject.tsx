// Add a project, before the confirmation: two ways in (a project already in iii cloud, or a new one), the list of
// iii cloud's projects, and the folder step. Nothing is read until the person picks a folder; nothing is created
// before the confirmation's button. The confirmation itself is Setup's.

import type { Host } from '@iii-dev/console-ui'
import { Button, DirectoryPicker, Input, List, ListItem } from '@iii-dev/console-ui'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import { useCloud } from './api'
import { hubOf, useActions } from './hub'
import { plural } from './model'
import { Notice, StepLink } from './parts'
import type { Project, Status, WorkerError } from './types'

/** A project of iii cloud, as the team's list has it. */
export type Listed = {
  id: string
  name: string
  repository: string
  team: { id: string; name: string }
  environments: number | null
}

type Raw = {
  id: string
  name: string
  repository?: { full_name?: string } | null
  environments?: unknown[]
  delete_after?: string | null
  status?: string
}

type Read = { projects: Listed[] | null; error: WorkerError | null }

/** Reads one team's projects (`GET /teams/{id}/projects`, the page's read-only proxy) and hands them up. */
function TeamReader({
  host,
  status,
  team,
  onRead,
  retry,
}: {
  host: Host
  status: Status
  team: { id: string; name: string }
  onRead: (id: string, r: Read) => void
  /** Moves when the person presses "Try again". */
  retry: number
}) {
  const c = useCloud<{ projects: Raw[] }>(host, status, null, `/teams/${team.id}/projects`)
  const again = c.refetch
  useEffect(() => {
    if (retry) again()
  }, [retry])
  useEffect(() => {
    const projects = c.data
      ? c.data.projects
          .filter((p) => !p.delete_after && p.status !== 'deleting' && p.status !== 'deleted')
          .map((p) => ({
            id: p.id,
            name: p.name,
            repository: p.repository?.full_name ?? '',
            team,
            environments: Array.isArray(p.environments) ? p.environments.length : null,
          }))
      : null
    onRead(team.id, { projects, error: c.error })
  }, [c.data, c.error, team, onRead])
  return null
}

/** The projects of every team the person is in; a team whose read failed is in `failed`, never counted as empty. */
export function useAccountProjects(status: Status) {
  const [reads, setReads] = useState<Record<string, Read>>({})
  const [retry, setRetry] = useState(0)
  const teams = status.connection.teams
  const onRead = useCallback((id: string, r: Read) => setReads((o) => ({ ...o, [id]: r })), [])
  const answered = (id: string) => !!(reads[id]?.projects || reads[id]?.error)
  const loaded = teams.every((t) => answered(t.id))
  const projects = teams.flatMap((t) => reads[t.id]?.projects ?? [])
  const failed = teams.filter((t) => !reads[t.id]?.projects && reads[t.id]?.error)
  const readers = (host: Host) =>
    teams.map((t) => <TeamReader key={t.id} host={host} status={status} team={t} onRead={onRead} retry={retry} />)
  return {
    projects,
    loaded,
    failed,
    error: failed.length ? (reads[failed[0].id]?.error ?? null) : null,
    teams: teams.length,
    retry: () => setRetry(retry + 1),
    readers,
  }
}

/** What went wrong reading the projects, as a notice with "Try again" (null when every team answered). */
export function ReadFailure({
  failed,
  teams,
  error,
  retry,
}: {
  failed: { name: string }[]
  teams: number
  error: WorkerError | null
  retry: () => void
}) {
  if (!failed.length || !error) return null
  const who = teams > 1 ? failed.map((t) => `${t.name}'s`).join(' and ') : "iii cloud's"
  return (
    <Notice
      variant="warn"
      headline={`${who} projects could not be read`}
      detail={error.message}
      action={
        <Button size="sm" variant="secondary" onClick={retry}>
          Try again
        </Button>
      }
    />
  )
}

function Head({ title, lead }: { title: string; lead: ReactNode }) {
  return (
    <div className="iii-cloud-setup-head">
      <h2 className="iii-cloud-setup-title">{title}</h2>
      <p className="iii-cloud-setup-lead">{lead}</p>
    </div>
  )
}

/** The two ways in, as framed choices, and the way back to Projects. */
export function AddStart({
  existing,
  failure,
  onExisting,
  onNew,
  cancel,
}: {
  /** Why option 1 is off, else null. */
  existing: string | null
  /** The read of iii cloud's projects failed: what, and "Try again". */
  failure?: ReactNode
  onExisting: () => void
  onNew: () => void
  cancel: () => void
}) {
  return (
    <>
      <Head title="Add a project" lead="Choose how to start. You confirm everything before anything is created." />
      <List aria-label="How to add a project" className="iii-cloud-add-choices">
        <ListItem
          className="iii-cloud-choice"
          disabled={existing !== null}
          label="Use a project already in iii cloud"
          description={
            existing ?? 'Pick the project, then its folder on this machine. Add an environment or deploy from here.'
          }
          onClick={onExisting}
        />
        <ListItem
          className="iii-cloud-choice"
          label="Create a new project in iii cloud"
          description="Pick a folder on this machine. The page reads its repository and creates the project with its first environment."
          onClick={onNew}
        />
      </List>
      {failure}
      <p className="iii-cloud-desc">
        Either way you confirm the folder, the environment and the compose file, then the check starts. Nothing changes
        in iii cloud before that.
      </p>
      <div className="iii-cloud-row">
        <StepLink onClick={cancel}>Cancel</StepLink>
      </div>
    </>
  )
}

const MANY = 8

/** "Which project?": one list of iii cloud's projects, with a filter when there are many. */
export function ProjectList({
  projects,
  loaded,
  failure,
  onPick,
  onBack,
}: {
  projects: Listed[]
  loaded: boolean
  /** A team whose projects could not be read, with "Try again". */
  failure?: ReactNode
  onPick: (p: Listed) => void
  onBack: () => void
}) {
  const [q, setQ] = useState('')
  const needle = q.trim().toLowerCase()
  const shown = needle ? projects.filter((p) => `${p.name} ${p.repository}`.toLowerCase().includes(needle)) : projects
  const teams = new Set(projects.map((p) => p.team.id)).size
  return (
    <>
      <Head
        title="Which project?"
        lead="The projects of your teams in iii cloud. Next you pick its folder on this machine."
      />
      {failure}
      {projects.length > MANY ? (
        <Input aria-label="Filter projects" placeholder="Filter by name or repository" value={q} onChange={setQ} />
      ) : null}
      {!loaded && !projects.length ? (
        <p className="iii-cloud-desc">Reading your projects…</p>
      ) : (
        <List aria-label="Projects in iii cloud" className="iii-cloud-add-choices">
          {shown.map((p) => (
            <ListItem
              key={p.id}
              className="iii-cloud-choice"
              label={p.name}
              description={[
                p.repository ? `Built from ${p.repository}` : null,
                p.environments != null ? plural(p.environments, 'environment') : null,
                teams > 1 ? `team ${p.team.name}` : null,
              ]
                .filter(Boolean)
                .join(' · ')}
              onClick={() => onPick(p)}
            />
          ))}
        </List>
      )}
      {loaded && needle && !shown.length ? <p className="iii-cloud-desc">No project matches “{q}”.</p> : null}
      <div className="iii-cloud-row">
        <StepLink onClick={onBack}>Back</StepLink>
      </div>
    </>
  )
}

/** What the worker's `AGENT_COPY` and `REPOSITORY_MISMATCH` refusals say, in place, with the way on. */
function Refusal({ error, onPath }: { error: WorkerError; onPath: (dir: string) => void }) {
  const main = error.code === 'AGENT_COPY' ? error.fix : null
  return (
    <Notice
      variant="warn"
      headline={error.message}
      action={
        main ? (
          <Button size="sm" variant="secondary" onClick={() => onPath(main)}>
            Use {main}
          </Button>
        ) : undefined
      }
    />
  )
}

/**
 * The folder step: the picker alone. On a pick the page reads the folder (`project::resolve`, with the
 * project when one was chosen) and hands the answer up once the status holds it. A refusal stays here.
 */
export function FolderStep({
  host,
  title,
  lead,
  projectId,
  defaultDir,
  onRead,
  onBack,
}: {
  host: Host
  title: string
  lead: string
  projectId?: string
  defaultDir?: string | null
  onRead: (project: Project) => void
  onBack: () => void
}) {
  const actions = useActions(host)
  const [path, setPath] = useState<string | null>(null)
  // The folder being read: an answer for any other (after Back, another pick, or leaving) is dropped.
  const reading = useRef<string | null>(null)
  useEffect(() => () => void (reading.current = null), [])
  const err = actions.error
  const read = (dir: string) => {
    reading.current = dir
    setPath(dir)
    void actions
      .call<Project>('resolve', 'iii-cloud::project::resolve', {
        path: dir,
        ...(projectId ? { cloud_project_id: projectId } : {}),
      })
      .then(async (out) => {
        if (!out || reading.current !== dir) return
        await hubOf(host).refresh(false)
        if (reading.current === dir) onRead(out)
      })
  }
  const shown = err && ['AGENT_COPY', 'REPOSITORY_MISMATCH'].includes(err.code) ? err : null
  return (
    <>
      <Head title={title} lead={lead} />
      <div className="iii-cloud-detailcard">
        <div className="iii-cloud-detail-open">
          <span className="iii-cloud-setup-label">Folder</span>
          <DirectoryPicker value={path} defaultDir={defaultDir ?? null} emptyLabel="Choose folder" onChange={read} />
          {actions.busy === 'resolve' ? <span className="iii-cloud-desc">Reading the folder…</span> : null}
        </div>
      </div>
      {shown ? <Refusal error={shown} onPath={read} /> : err ? <Notice variant="alert" headline={err.message} /> : null}
      <div className="iii-cloud-row">
        <StepLink
          onClick={() => {
            reading.current = null
            onBack()
          }}
        >
          Back
        </StepLink>
      </div>
    </>
  )
}

/** The repository already has a project in iii cloud: say so, and let the person choose. */
export function Already({
  project,
  onUse,
  onAnother,
  onBack,
}: {
  project: Project
  onUse: () => void
  onAnother: () => void
  onBack: () => void
}) {
  const name = project.cloud_project?.name ?? ''
  return (
    <>
      <Head title="Add a project" lead="This folder's repository is already in iii cloud." />
      <Notice variant="info" headline={`${project.repository} is already ${name} in iii cloud.`} />
      <div className="iii-cloud-row">
        <Button variant="primary" onClick={onUse}>
          Use {name}
        </Button>
        <Button variant="secondary" onClick={onAnother}>
          Create another project
        </Button>
        <StepLink onClick={onBack}>Back</StepLink>
      </div>
    </>
  )
}

import type { Host } from '@iii-dev/console-ui'
import { Button, Checkbox, Input, ListItem, Select } from '@iii-dev/console-ui'
import { Circle, CircleDot } from 'lucide-react'
import { useState } from 'react'
import { useActions } from './hub'
import { composeFile, plainDetail, plainMessage } from './model'
import { Notice, Spinner } from './parts'
import type { CloudProjectChoice, Status, WorkerError } from './types'

/** `project::create` and `project::create_environment`: the projects read fresh and the ids to select. */
export type Created = {
  cloud_projects?: CloudProjectChoice[]
  project_id?: string | null
  environment_id?: string | null
}

/**
 * A create's error in plain words. A refusal by role keeps iii cloud's own sentence, which the worker puts in
 * parentheses (`create_error`): the page's role hint is only a hint, the cloud's answer is the rule.
 */
function CreateError({ error }: { error: WorkerError }) {
  const cloud = error.code === 'TEAM_FORBIDDEN' ? error.message.match(/\(([^()]+)\)\s*$/)?.[1] : undefined
  return (
    <Notice
      variant="alert"
      headline={plainMessage(error)}
      detail={cloud ? `iii cloud says: ${cloud}` : plainDetail(error)}
    />
  )
}

const field = (id: string, label: string, control: React.ReactNode) => (
  <div className="iii-cloud-formrow">
    <label className="iii-cloud-lbl" htmlFor={id}>
      {label}
    </label>
    {control}
  </div>
)

/** A new project in iii cloud for this repository: team, compose file, first environment. */
export function NewProjectForm({
  host,
  status,
  path,
  onCreated,
  onCancel,
}: {
  host: Host
  status: Status
  path: string
  onCreated: (out: Created, environmentName: string) => void
  onCancel?: () => void
}) {
  const { busy, error, call } = useActions(host)
  const teams = status.connection.teams
  const manifests = (status.project?.importable?.manifests ?? []).filter((m) => m.choosable)
  const [team, setTeam] = useState(status.project?.team?.id ?? teams[0]?.id ?? '')
  const [manifest, setManifest] = useState(manifests[0]?.path ?? 'worker-compose.yaml')
  const [name, setName] = useState('production')
  const [production, setProduction] = useState(true)
  const creating = busy === 'create'
  const create = async () => {
    const out = await call<Created>('create', 'iii-cloud::project::create', {
      path,
      team_id: team,
      manifest,
      environment_name: name.trim(),
      production,
    })
    if (out !== null) onCreated(out ?? {}, name.trim())
  }
  return (
    <fieldset className="iii-cloud-form" aria-label="New project in iii cloud">
      {teams.length > 1
        ? field(
            'iii-cloud-np-team',
            'Team',
            <Select
              id="iii-cloud-np-team"
              aria-label="Team"
              value={team}
              options={teams.map((t) => ({ value: t.id, label: t.name }))}
              onChange={setTeam}
            />,
          )
        : null}
      {field(
        'iii-cloud-np-manifest',
        'Compose file',
        manifests.length > 1 ? (
          <Select
            id="iii-cloud-np-manifest"
            aria-label="Compose file"
            value={manifest}
            options={manifests.map((m) => ({ value: m.path, label: m.path }))}
            onChange={setManifest}
          />
        ) : (
          <Input id="iii-cloud-np-manifest" className="iii-cloud-mono" value={manifest} onChange={setManifest} />
        ),
      )}
      {field('iii-cloud-np-env', 'Environment name', <Input id="iii-cloud-np-env" value={name} onChange={setName} />)}
      <Checkbox label="Production environment" checked={production} onChange={(e) => setProduction(e.target.checked)} />
      {error ? <CreateError error={error} /> : null}
      <div className="iii-cloud-row">
        <Button variant="primary" disabled={creating || !name.trim() || !team} onClick={create}>
          {creating ? (
            <>
              <Spinner />
              Creating…
            </>
          ) : (
            'Create project'
          )}
        </Button>
        {onCancel ? (
          <Button variant="secondary" disabled={creating} onClick={onCancel}>
            Cancel
          </Button>
        ) : null}
      </div>
    </fieldset>
  )
}

/** The environment name rule of iii cloud (`valid_env_name`): checked here so the person sees it before Create. */
const validName = (n: string) => n.length <= 30 && /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(n)
/** The "Start empty" choice: environment ids are UUIDs, so it never names one. */
const START_EMPTY = 'start-empty'
type Choice = { value: string; label: string; description: string }

/** Radio rows: one Tab stop, the arrows move the choice. */
function Choices({
  labelledBy,
  value,
  options,
  onChange,
}: {
  labelledBy: string
  value: string
  options: Choice[]
  onChange: (v: string) => void
}) {
  const STEP: Record<string, number> = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }
  const move = (e: React.KeyboardEvent<HTMLElement>, at: number) => {
    const step = STEP[e.key]
    if (!step) return
    e.preventDefault()
    const next = (at + step + options.length) % options.length
    onChange(options[next].value)
    e.currentTarget.parentElement?.querySelectorAll<HTMLElement>('[role=radio]')[next]?.focus()
  }
  return (
    <div className="iii-cloud-ne-choices" role="radiogroup" aria-labelledby={labelledBy}>
      {options.map((o, i) => {
        const on = o.value === value
        return (
          <ListItem
            key={o.value}
            role="radio"
            aria-checked={on}
            // A framed card, an ink border when chosen (`aria-checked`): `selected` would add aria-pressed.
            className="iii-cloud-choice"
            tabIndex={on ? 0 : -1}
            leading={on ? <CircleDot /> : <Circle />}
            label={o.label}
            description={o.description}
            onClick={() => onChange(o.value)}
            onKeyDown={(e) => move(e, i)}
          />
        )
      })}
    </div>
  )
}

/**
 * A new environment in an existing cloud project (Mng8): name, the branch it deploys from, and where its
 * settings and secrets come from (an environment, or none). The compose file is `baseManifest`'s.
 */
export function NewEnvironmentForm({
  host,
  project,
  defaultBranch,
  baseManifest,
  copyFrom,
  onCreated,
  onCancel,
  baseBranch,
  preview,
}: {
  host: Host
  project: CloudProjectChoice
  /** The branch field's first value. */
  defaultBranch: string
  baseManifest: string | null | undefined
  copyFrom: { id: string; name: string }[]
  onCreated: (out: Created, name: string) => void
  onCancel?: () => void
  /** The repository's default branch: with it, the branch field says how the environment deploys. */
  baseBranch?: string | null
  /** What iii cloud will create for the chosen source; `null` is Start empty. */
  preview?: (from: { id: string; name: string } | null) => React.ReactNode
}) {
  const { busy, error, call } = useActions(host)
  const [name, setName] = useState('')
  const [branch, setBranch] = useState(defaultBranch)
  const prod = new Set(project.environments?.filter((e) => e.production).map((e) => e.id))
  // Non-production first: a trial copies one of those, so production's secrets stay in production.
  const sources = [...copyFrom].sort((a, b) => Number(prod.has(a.id)) - Number(prod.has(b.id)))
  const trial = sources.find((e) => !prod.has(e.id))
  const [copy, setCopy] = useState(sources[0]?.id ?? '')
  const creating = busy === 'create'
  const n = name.trim()
  const b = branch.trim()
  const badName = n !== '' && !validName(n)
  const how =
    !baseBranch || !b
      ? null
      : b === baseBranch
        ? `It deploys when a pull request into ${b} is merged.`
        : 'This machine deploys it after the check. A push from anywhere else deploys right away.'
  const choices: Choice[] = [
    ...sources.map((e) => ({
      value: e.id,
      label: e.name,
      description: prod.has(e.id)
        ? 'copies production secrets'
        : e.id === trial?.id
          ? 'recommended for a trial'
          : 'copies its settings and secrets',
    })),
    { value: START_EMPTY, label: 'Start empty', description: 'you set everything' },
  ]
  const create = async () => {
    const out = await call<Created>('create', 'iii-cloud::project::create_environment', {
      cloud_project_id: project.id,
      name: n,
      branch: b,
      manifest: composeFile(baseManifest),
      ...(copy === START_EMPTY ? { start_empty: true } : copy ? { copy_from: copy } : {}),
    })
    if (out !== null) onCreated(out ?? {}, n)
  }
  return (
    <fieldset className="iii-cloud-form iii-cloud-ne-form" aria-label="New environment in iii cloud">
      {field(
        'iii-cloud-ne-name',
        'Name',
        <>
          <Input
            id="iii-cloud-ne-name"
            value={name}
            onChange={setName}
            aria-invalid={badName || undefined}
            aria-describedby={badName ? 'iii-cloud-ne-name-hint' : undefined}
          />
          {badName ? (
            <span id="iii-cloud-ne-name-hint" className="iii-cloud-psmall">
              Lowercase letters, digits and hyphens, at most 30.
            </span>
          ) : null}
        </>,
      )}
      {field(
        'iii-cloud-ne-branch',
        'Deploys from',
        <>
          <Input
            id="iii-cloud-ne-branch"
            className="iii-cloud-mono"
            value={branch}
            onChange={setBranch}
            aria-describedby={how ? 'iii-cloud-ne-branch-hint' : undefined}
          />
          {how ? (
            <span id="iii-cloud-ne-branch-hint" className="iii-cloud-psmall">
              {how}
            </span>
          ) : null}
        </>,
      )}
      {sources.length > 0 ? (
        <div className="iii-cloud-formrow">
          <span id="iii-cloud-ne-copy" className="iii-cloud-lbl">
            Copy settings and secrets from
          </span>
          <Choices labelledBy="iii-cloud-ne-copy" value={copy} options={choices} onChange={setCopy} />
        </div>
      ) : null}
      {preview?.(copy === START_EMPTY ? null : (sources.find((e) => e.id === copy) ?? null))}
      {error ? <CreateError error={error} /> : null}
      <div className="iii-cloud-row">
        <Button variant="primary" disabled={creating || !n || badName || !b} onClick={create}>
          {creating ? (
            <>
              <Spinner />
              Creating…
            </>
          ) : (
            `Create ${n || 'environment'}`
          )}
        </Button>
        {onCancel ? (
          <Button variant="secondary" disabled={creating} onClick={onCancel}>
            Cancel
          </Button>
        ) : null}
        <span className="iii-cloud-psmall">
          Nothing is deployed yet: its first deploy goes through Check before deploy.
        </span>
      </div>
    </fieldset>
  )
}

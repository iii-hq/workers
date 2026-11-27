// The fields of the confirm screen's Project and Environment rows when the choice is "New project" or "New
// environment", and the calls that make them. Nothing is created here: the confirm button does it, in order.

import { Checkbox, Input, ListItem, Select } from '@iii-dev/console-ui'
import { Circle, CircleDot } from 'lucide-react'
import { composeFile, deployWay, plainDetail, plainMessage } from './model'
import { Notice } from './parts'
import type { CloudProjectChoice, WorkerError } from './types'

/** `project::create` and `project::create_environment`: the projects read fresh and the ids to select. */
export type Created = {
  cloud_projects?: CloudProjectChoice[]
  project_id?: string | null
  environment_id?: string | null
}

type Call = <T>(key: string, id: string, payload?: Record<string, unknown>) => Promise<T | null>

/**
 * A create's error in plain words. A refusal by role keeps iii cloud's own sentence, which the worker puts in
 * parentheses (`create_error`): the page's role hint is only a hint, the cloud's answer is the rule.
 */
export function CreateError({ error }: { error: WorkerError }) {
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

/** A new project's own choices: its team, and the first environment's name and kind. */
export type NewProject = { team: string; env: string; production: boolean }

export function NewProjectFields({
  part,
  teams,
  value,
  onChange,
}: {
  /** The Project row asks for the team; the Environment row for the first environment. */
  part: 'team' | 'env'
  teams: { id: string; name: string }[]
  value: NewProject
  onChange: (v: NewProject) => void
}) {
  if (part === 'team' && teams.length < 2) return null
  return (
    <fieldset className="iii-cloud-form" aria-label="New project in iii cloud">
      {part === 'team'
        ? field(
            'iii-cloud-np-team',
            'Team',
            <Select
              id="iii-cloud-np-team"
              aria-label="Team"
              value={value.team}
              options={teams.map((t) => ({ value: t.id, label: t.name }))}
              onChange={(team: string) => onChange({ ...value, team })}
            />,
          )
        : null}
      {part === 'env' ? (
        <>
          {field(
            'iii-cloud-np-env',
            'Name',
            <Input id="iii-cloud-np-env" value={value.env} onChange={(env) => onChange({ ...value, env })} />,
          )}
          <Checkbox
            label="Production environment"
            checked={value.production}
            onChange={(e) => onChange({ ...value, production: e.target.checked })}
          />
        </>
      ) : null}
    </fieldset>
  )
}

/** `project::create`: the project and its first environment. */
export const createProject = (call: Call, path: string, manifest: string, v: NewProject) =>
  call<Created>('create', 'iii-cloud::project::create', {
    path,
    team_id: v.team,
    manifest,
    environment_name: v.env.trim(),
    production: v.production,
  })

/** The environment name rule of iii cloud (`valid_env_name`): checked here so the person sees it before Create. */
const validName = (n: string) => n.length <= 30 && /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(n)
/** The "Start empty" choice: environment ids are UUIDs, so it never names one. */
const START_EMPTY = 'start-empty'
type Choice = { value: string; label: string; description: string }

/** A new environment's own choices: its name, the branch it deploys from, and where its settings come from. */
export type NewEnv = { name: string; branch: string; copy: string }

/** The environments a new one can copy from, non-production first: a trial copies one of those. */
function copySources(project: CloudProjectChoice, copyFrom: { id: string; name: string }[]) {
  const prod = new Set(project.environments?.filter((e) => e.production).map((e) => e.id))
  const sources = [...copyFrom].sort((a, b) => Number(prod.has(a.id)) - Number(prod.has(b.id)))
  return { prod, sources, trial: sources.find((e) => !prod.has(e.id)) }
}

/** The new environment can be created: a name iii cloud accepts and a branch. */
export const newEnvReady = (v: NewEnv) => v.name.trim() !== '' && validName(v.name.trim()) && v.branch.trim() !== ''

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

export function NewEnvironmentFields({
  project,
  copyFrom,
  baseBranch,
  value,
  onChange,
}: {
  project: CloudProjectChoice
  copyFrom: { id: string; name: string }[]
  /** The repository's default branch: with it, the branch field says whether the environment gets its own branch. */
  baseBranch?: string | null
  value: NewEnv
  onChange: (v: NewEnv) => void
}) {
  const { prod, sources, trial } = copySources(project, copyFrom)
  const n = value.name.trim()
  const b = value.branch.trim()
  const badName = n !== '' && !validName(n)
  // A new environment is never production here: it gets an assisted deploy, on its own branch when this one is shared.
  const way = deployWay(baseBranch ?? null, { name: n || '…', production: false, branch: b }, project.environments ?? [])
  const how = !b
    ? null
    : way.way === 'assisted' && way.own
      ? `Its first assisted deploy gives it its own branch, ${way.own}. ${b} is never pushed.`
      : `An assisted deploy sends the checked commit to ${b}. A push to ${b} from anywhere else deploys without the check.`
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
  return (
    <fieldset className="iii-cloud-form iii-cloud-ne-form" aria-label="New environment in iii cloud">
      {field(
        'iii-cloud-ne-name',
        'Name',
        <>
          <Input
            id="iii-cloud-ne-name"
            value={value.name}
            onChange={(name) => onChange({ ...value, name })}
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
        'Branch',
        <>
          <Input
            id="iii-cloud-ne-branch"
            className="iii-cloud-mono"
            value={value.branch}
            onChange={(branch) => onChange({ ...value, branch })}
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
          <Choices
            labelledBy="iii-cloud-ne-copy"
            value={value.copy || sources[0].id}
            options={choices}
            onChange={(copy) => onChange({ ...value, copy })}
          />
        </div>
      ) : null}
    </fieldset>
  )
}

/** `project::create_environment`, in the compose file picked. */
export function createEnvironment(
  call: Call,
  project: CloudProjectChoice,
  copyFrom: { id: string; name: string }[],
  manifest: string | null | undefined,
  v: NewEnv,
) {
  const { sources } = copySources(project, copyFrom)
  const copy = v.copy || sources[0]?.id || ''
  return call<Created>('create', 'iii-cloud::project::create_environment', {
    cloud_project_id: project.id,
    name: v.name.trim(),
    branch: v.branch.trim(),
    manifest: composeFile(manifest),
    ...(copy === START_EMPTY ? { start_empty: true } : copy ? { copy_from: copy } : {}),
  })
}

import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  ConfirmDialog,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  EmptyState,
  Input,
  List,
  ListItem,
  Panel,
  PanelBody,
  PanelHeader,
  SegmentedControl,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { errorCode, errorMessage } from '@iii-dev/console-ui/format'
import { usePaneState } from '@iii-dev/console-ui/hooks'
import { Circle, CircleDot } from 'lucide-react'
import { Fragment, type ReactNode, useId, useState } from 'react'
import { type Cloud, cloudWrite, useCloud } from './api'
import { ConfigImportDialog, fieldLines } from './ConfigImport'
import { useActions } from './hub'
import { ago, appFile, COMPOSE_ISSUE, composeFile, configDir, envPageUrl, plainMessage, servingDeploy } from './model'
import { ErrorPanel, LinkButton, Mono, Notice, StatusChip } from './parts'
import type { EnvTabProps, Route } from './route'
import { SecretEntry } from './SecretEntry'
import type { ComposeFile, SecretRequest, Status, WorkerError } from './types'
import { useValues, ValuesEditor } from './ValuesEditor'

/* ── what iii cloud answers (cloud-v1.yaml: Environment, Variable, Resource) ── */

type EnvBody = { manifest: string | null; settings?: Record<string, unknown> | null; from_file?: string[] }
type CloudVariable = {
  name: string
  type: 'secret' | 'variable' | 'builtin'
  value: string | null
  status: 'set' | 'missing' | 'empty_default'
  required_by: string[]
  applies: string | null
  updated_at: string | null
}
type Resource = {
  id: string
  kind: string
  alias: string
  mode: 'managed' | 'external'
  tier?: string | null
  connection?: { name: string }[]
  credentials: { rotated_at: string | null; rotating: boolean } | null
  created_at: string
}
type Override = NonNullable<Status['check']>['plan']['confirmations']['overrides'][number]
/** `GET …/config` (cloud-v1.yaml ConfigView): every worker's fields; `ui` ones were set in iii cloud. */
type ConfigView = {
  version: number
  draft_changed: boolean
  workers: {
    name: string
    fields: { key: string; value: unknown; source: 'default' | 'manifest' | 'managed' | 'ui' }[]
  }[]
}

type Ctx = { host: Host; status: Status; envId: string; envName: string }
type Ask = { title: string; description: string; label: string; tone?: 'danger'; run: () => Promise<void> }

/** A `cloud:` block key (what a plan override names) in words, and the environment setting that holds it. */
const SETTING: Record<string, { label: string; noun: string; key: string | null }> = {
  release: { label: 'Release command', noun: 'release command', key: 'release' },
  web: { label: 'Web traffic', noun: 'web traffic setting', key: 'web' },
  healthcheck: { label: 'Health check', noun: 'health check', key: 'healthcheck' },
  build: { label: 'Build command', noun: 'build command', key: 'build_command' },
  runtimes: { label: 'Runtimes', noun: 'runtimes', key: 'runtimes' },
  root_directory: { label: 'Root directory', noun: 'root directory', key: 'root_directory' },
  groups: { label: 'Machine groups', noun: 'machine groups', key: null },
}

/** A resource kind: the row's label and what it is. */
const RESOURCE: Record<string, [string, string]> = {
  database: ['Database', 'Postgres'],
  postgres: ['Database', 'Postgres'],
  storage: ['Storage', 'object storage'],
  queue: ['Queue', 'RabbitMQ'],
  state: ['State', 'Redis'],
}

const and = (xs: string[]) => (xs.length < 2 ? (xs[0] ?? '') : `${xs.slice(0, -1).join(', ')} and ${xs.at(-1)}`)
const enc = encodeURIComponent

/** `after_restart:app,jobs` → the groups that read the value when they restart. */
const groupsOf = (applies: string | null | undefined) =>
  applies?.startsWith('after_restart:') ? applies.slice('after_restart:'.length).split(',').filter(Boolean) : []

/** A setting's value in the block's shape, in one line. */
function show(v: unknown): string {
  if (v == null) return 'nothing'
  if (typeof v !== 'object') return String(v)
  const o = v as Record<string, unknown>
  if (typeof o.command === 'string') return o.command
  if (typeof o.path === 'string') return o.timeout ? `${o.path} · timeout ${o.timeout}` : o.path
  if (o.serve === 'container') return `the ${o.container ?? 'detected'} container${o.port ? `, port ${o.port}` : ''}`
  if (o.serve === 'static') return `static files from ${o.directory ?? 'the build'}`
  if (o.serve === 'none') return 'no web part'
  return Object.entries(o)
    .map(([k, x]) => `${k}: ${show(x)}`)
    .join(', ')
}

/** One cloud write at a time per section: the busy key and the cloud's error, its 403 message kept. */
function useWrite() {
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<WorkerError | null>(null)
  const run = async (key: string, write: () => Promise<unknown>) => {
    setBusy(key)
    setError(null)
    try {
      await write()
      return true
    } catch (err) {
      setError({ code: errorCode(err) ?? 'ERROR', message: errorMessage(err) })
      return false
    } finally {
      setBusy(null)
    }
  }
  return { busy, error, run }
}

const overrideKey = (envId: string, o: Override) => `override:${envId}:${o.setting}:${JSON.stringify(o.console)}`
const restartKey = (envId: string, v: CloudVariable) => `restart:${envId}:${v.name}:${v.updated_at}`

/**
 * An environment's Settings tab (Mng3) and the change-file dialog (Mng11): the values the app file reads
 * (`ValuesEditor`, the agent's asks among them), console values that differ from the app file, what the next
 * deploy runs, and the other secrets and variables. Every change here is the person's: the file's values go
 * through `iii-cloud::values::save`, other variables and settings through `api::write`, other secret values
 * only through `iii-cloud::secret::provide` (`SecretEntry`).
 */
export function EnvSettings({ host, status, go, narrow, paneKey, envId }: EnvTabProps) {
  const env = useCloud<EnvBody>(host, status, envId, '')
  const vars = useCloud<{ variables: CloudVariable[] }>(host, status, envId, '/variables')
  const res = useCloud<{ resources: Resource[] }>(host, status, envId, '/resources')
  const [dismissed, setDismissed] = usePaneState<string[]>(`iii-cloud:settings-dismissed:${paneKey}`, [])
  const dismiss = (...keys: string[]) => setDismissed((d) => [...d, ...keys])
  const [confirm, setConfirm] = useState<Ask | null>(null)
  const [changing, setChanging] = useState(false)
  const values = useValues(host, status, {})
  const c: Ctx = { host, status, envId, envName: status.project?.environment?.name ?? 'this environment' }
  const check = status.check?.environment_id === envId ? status.check : null
  const overrides = check?.plan.confirmations.overrides ?? []
  return (
    <div className="iii-cloud-set">
      {values.data || values.error ? (
        <Panel aria-label="Values your app file reads">
          <PanelHeader className="iii-cloud-panel-head">Values your app file reads</PanelHeader>
          <PanelBody className="iii-cloud-set-body">
            <ValuesEditor host={host} status={status} mode="settings" narrow={narrow} read={values} />
          </PanelBody>
        </Panel>
      ) : null}
      <Asked c={c} />
      <Overrides c={c} env={env} overrides={overrides} dismissed={dismissed} dismiss={dismiss} ask={setConfirm} />
      <div className="iii-cloud-mng-grid">
        <div className="iii-cloud-set-col">
          <NextDeploy
            c={c}
            env={env}
            res={res}
            overridden={new Set(overrides.map((o) => o.setting))}
            narrow={narrow}
            onChange={() => setChanging(true)}
          />
          <WorkersConfig c={c} />
          <Restart c={c} vars={vars} dismissed={dismissed} dismiss={dismiss} ask={setConfirm} />
        </div>
        <Variables
          c={c}
          vars={vars}
          res={res}
          narrow={narrow}
          ask={setConfirm}
          read={new Set(values.data?.items.map((i) => i.name))}
        />
      </div>
      {changing ? <ChangeFile c={c} go={go} onClose={() => setChanging(false)} /> : null}
      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(open) => (open ? undefined : setConfirm(null))}
        title={confirm?.title ?? ''}
        description={confirm?.description}
        confirmLabel={confirm?.label}
        tone={confirm?.tone}
        onConfirm={() => confirm?.run()}
      />
    </div>
  )
}

/* ── what became of the agent's asks: the asks themselves are rows of the values editor ── */

function Asked({ c }: { c: Ctx }) {
  const mine = c.status.secrets.filter((r) => r.environment_id === c.envId)
  return (
    <>
      {mine
        .filter((r) => r.state === 'set' && r.local === 'skipped')
        .map((r) => (
          <Notice
            key={r.id}
            variant="info"
            headline={`${r.name}: not written to .env`}
            detail="Not written to .env: the file is tracked by git"
          />
        ))}
      {mine.map((r) =>
        r.state === 'failed' && r.error ? (
          <ErrorPanel
            key={r.id}
            error={{ ...r.error, message: `${r.name}: ${plainMessage(r.error)}`, code: 'SECRET' }}
          />
        ) : null,
      )}
    </>
  )
}

/* ── console values the app file would replace ──────────────────────── */

function Overrides({
  c,
  env,
  overrides,
  dismissed,
  dismiss,
  ask,
}: {
  c: Ctx
  env: Cloud<EnvBody>
  overrides: Override[]
  dismissed: string[]
  dismiss: (...keys: string[]) => void
  ask: (a: Ask) => void
}) {
  // The check is older than the environment: a console value cleared since then is no conflict.
  const open = overrides.filter((o) => {
    const key = SETTING[o.setting]?.key
    const cleared = !!env.data && !!key && env.data.settings?.[key] == null && o.console != null
    return !cleared && !dismissed.includes(overrideKey(c.envId, o))
  })
  return (
    <>
      {open.map((o) => {
        const s = SETTING[o.setting] ?? { label: o.setting, noun: o.setting, key: null }
        const key = s.key
        return (
          <Notice
            key={o.setting}
            variant="warn"
            headline={`The ${s.noun} on ${c.envName} ${s.noun.endsWith('s') ? 'were' : 'was'} changed in the iii cloud console`}
            detail={
              <>
                iii cloud console: <Mono>{show(o.console)}</Mono> · your app file: <Mono>{show(o.file)}</Mono>.{' '}
                {key ? 'Using your file removes the console value. ' : ''}Keeping it hides this note: the review before
                your next deploy shows the difference again.
              </>
            }
            action={
              <span className="iii-cloud-row">
                {key ? (
                  <Button
                    size="sm"
                    variant="primary"
                    onClick={() =>
                      ask({
                        title: `Use your app file's ${s.noun} on ${c.envName}?`,
                        description: `iii cloud removes the console's value, ${show(o.console)}. The next deploy of ${c.envName} runs your app file's, ${show(o.file)}.`,
                        label: 'Use my app file',
                        run: async () => {
                          await cloudWrite(c.host, c.status, c.envId, 'PATCH', '', { settings: { [key]: null } })
                          env.refetch()
                        },
                      })
                    }
                  >
                    Use my app file (recommended)
                  </Button>
                ) : null}
                <Button size="sm" variant="secondary" onClick={() => dismiss(overrideKey(c.envId, o))}>
                  Not now
                </Button>
              </span>
            }
          />
        )
      })}
    </>
  )
}

/* ── what the next deploy runs ──────────────────────────────────────── */

type SettingRow = { label: string; value: ReactNode; where: ReactNode }

function settingRows(env: EnvBody | null, overridden: Set<string>): SettingRow[] {
  if (!env) return []
  return Object.entries(SETTING).flatMap(([block, s]) => {
    const v = s.key ? env.settings?.[s.key] : null
    if (v == null || (typeof v === 'object' && Object.keys(v).length === 0)) return []
    const where = overridden.has(block) ? (
      <StatusChip tone="warning">iii cloud console</StatusChip>
    ) : env.from_file?.includes(block) ? (
      'app file'
    ) : (
      'iii cloud console'
    )
    return [{ label: s.label, value: block === 'web' ? show(v) : <Mono>{show(v)}</Mono>, where }]
  })
}

/** The environment's resources, then the ones its latest check would create. */
function resourceRows(resources: Resource[], s: Status): SettingRow[] {
  const line = (r: { kind: string; alias: string; tier?: string | null }) =>
    [`${RESOURCE[r.kind]?.[1] ?? r.kind} ${r.alias}`, r.tier].filter(Boolean).join(', ')
  const made = resources.map((r) => ({ kind: r.kind, text: line(r) }))
  const planned = (s.check?.plan.confirmations.resources ?? [])
    .filter((p) => !resources.some((r) => r.alias === p.alias))
    .map((p) => ({ kind: p.kind, text: `${line(p)}, made when you deploy` }))
  return [...made, ...planned].map((r) => ({
    label: RESOURCE[r.kind]?.[0] ?? r.kind,
    value: r.text,
    where: 'iii cloud',
  }))
}

function NextDeploy({
  c,
  env,
  res,
  overridden,
  narrow,
  onChange,
}: {
  c: Ctx
  env: Cloud<EnvBody>
  res: Cloud<{ resources: Resource[] }>
  overridden: Set<string>
  narrow: boolean
  onChange: () => void
}) {
  const current = composeFile(env.data?.manifest ?? c.status.project?.environment?.manifest)
  const checked = appFile(c.status)
  const canChange = (c.status.compose_files ?? []).some((f) => f.choosable && f.path !== current)
  const rows: SettingRow[] = [
    {
      label: 'App file',
      value: (
        <span className="iii-cloud-set-value">
          <Mono>{current}</Mono>
          {canChange ? (
            <button type="button" className="iii-cloud-linkbtn" onClick={onChange}>
              Change…
            </button>
          ) : null}
        </span>
      ),
      where: 'environment',
    },
    ...settingRows(env.data, overridden),
    ...resourceRows(res.data?.resources ?? [], c.status),
  ]
  return (
    <Panel aria-label="What the next deploy runs">
      <PanelHeader className="iii-cloud-panel-head">What the next deploy runs</PanelHeader>
      {env.error ? (
        <PanelBody>
          <Notice
            variant="warn"
            headline={`Couldn't read the settings of ${c.envName} from iii cloud`}
            detail={plainMessage(env.error)}
            action={
              <Button size="sm" variant="secondary" onClick={env.refetch}>
                Retry
              </Button>
            }
          />
        </PanelBody>
      ) : null}
      {env.loading && !env.data ? (
        <PanelBody>
          <Skeleton className="iii-cloud-set-skel" />
        </PanelBody>
      ) : narrow ? (
        <List aria-label="What the next deploy runs">
          {rows.map((r) => (
            <ListItem
              as="div"
              key={r.label}
              label={r.label}
              description={
                <span className="iii-cloud-set-value">
                  {r.value} · {r.where}
                </span>
              }
            />
          ))}
        </List>
      ) : (
        <TableViewport>
          <Table inset className="iii-cloud-table">
            <TableHeader>
              <TableRow>
                <TableHead>Setting</TableHead>
                <TableHead>Value</TableHead>
                <TableHead>Set in</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((r) => (
                <TableRow key={r.label}>
                  <TableCell>{r.label}</TableCell>
                  <TableCell>{r.value}</TableCell>
                  <TableCell>{r.where}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </TableViewport>
      )}
      {checked.switches ? (
        <PanelBody>
          <p className="iii-cloud-desc">
            The latest check used <Mono>{checked.path}</Mono>: {checked.note}.
          </p>
        </PanelBody>
      ) : null}
    </Panel>
  )
}

/* ── O4: the workers' config iii cloud keeps, and the copy of the local folder ── */

/**
 * The fields set in iii cloud, per worker, read-only; "Copy ./config again" opens the import dialog. Shown once
 * something is set, or while the check says the config folder isn't in iii cloud (LOCAL_CONFIG_NOT_SET).
 */
function WorkersConfig({ c }: { c: Ctx }) {
  const view = useCloud<ConfigView>(c.host, c.status, c.envId, '/config')
  const [importing, setImporting] = useState(false)
  const check = c.status.check?.environment_id === c.envId ? c.status.check : null
  const problem = check?.problems.find((p) => p.code === 'LOCAL_CONFIG_NOT_SET') ?? null
  const set = (view.data?.workers ?? [])
    .map((w) => ({ name: w.name, fields: w.fields.filter((f) => f.source === 'ui') }))
    .filter((w) => w.fields.length)
  if (!problem && !set.length) return null
  const dir = configDir(problem) ?? 'the config folder'
  const url = envPageUrl(c.status, 'config')
  return (
    <Panel aria-label="Workers' config">
      <PanelHeader className="iii-cloud-panel-head">Workers' config</PanelHeader>
      <PanelBody className="iii-cloud-set-body">
        {view.loading && !view.data ? (
          <Skeleton className="iii-cloud-set-skel" />
        ) : view.error ? (
          <Notice
            variant="warn"
            headline={`Couldn't read the workers' config of ${c.envName} from iii cloud`}
            detail={plainMessage(view.error)}
            action={
              <Button size="sm" variant="secondary" onClick={view.refetch}>
                Retry
              </Button>
            }
          />
        ) : set.length ? (
          <ul className="iii-cloud-ci-list" aria-label="Set in iii cloud">
            {set.map((w) => (
              <li key={w.name}>
                <b className="iii-cloud-ci-head">{w.name}</b>
                <pre className="iii-cloud-ci-fields">{fieldLines(w.fields)}</pre>
              </li>
            ))}
          </ul>
        ) : (
          <p className="iii-cloud-desc">{`Nothing copied yet: the workers start with their defaults on ${c.envName}. ${dir} isn't in the repository.`}</p>
        )}
        <span className="iii-cloud-row">
          <Button size="sm" variant="secondary" onClick={() => setImporting(true)}>
            {set.length ? `Copy ${dir} again` : `Copy ${dir} to iii cloud`}
          </Button>
          {url ? <LinkButton href={url}>Open in iii cloud</LinkButton> : null}
        </span>
      </PanelBody>
      {importing ? (
        <ConfigImportDialog
          host={c.host}
          status={c.status}
          envId={c.envId}
          env={c.envName}
          dir={dir}
          onClose={() => setImporting(false)}
          onApplied={view.refetch}
        />
      ) : null}
    </Panel>
  )
}

/* ── a value changed after the serving deploy started ───────────────── */

function Restart({
  c,
  vars,
  dismissed,
  dismiss,
  ask,
}: {
  c: Ctx
  vars: Cloud<{ variables: CloudVariable[] }>
  dismissed: string[]
  dismiss: (...keys: string[]) => void
  ask: (a: Ask) => void
}) {
  const serving = servingDeploy(c.status, c.envId)
  const since = serving ? Date.parse(serving.created_at) : Number.NaN
  const changed = (vars.data?.variables ?? []).filter(
    (v) =>
      groupsOf(v.applies).length > 0 &&
      !!v.updated_at &&
      Date.parse(v.updated_at) > since &&
      !dismissed.includes(restartKey(c.envId, v)),
  )
  if (!changed.length) return null
  const names = changed.map((v) => v.name)
  const groups = [...new Set(changed.flatMap((v) => groupsOf(v.applies)))]
  const many = groups.length > 1
  const done = () => dismiss(...changed.map((v) => restartKey(c.envId, v)))
  return (
    <Notice
      variant="warn"
      headline={`${and(names)} changed after the running deploy started`}
      detail={`${and(groups)} read ${names.length > 1 ? 'them' : 'it'} when ${many ? 'they restart' : 'it restarts'}. The next deploy restarts ${many ? 'them' : 'it'} too.`}
      action={
        <span className="iii-cloud-row">
          <Button
            size="sm"
            variant="secondary"
            onClick={() =>
              ask({
                title: `Restart ${and(groups)} on ${c.envName}?`,
                description: `${and(groups)} ${many ? 'restart and read' : 'restarts and reads'} the new ${names.length > 1 ? 'values' : 'value'}.`,
                label: 'Restart',
                run: async () => {
                  for (const g of groups) {
                    await cloudWrite(c.host, c.status, c.envId, 'POST', `/machines/groups/${enc(g)}/restart`)
                  }
                  done()
                },
              })
            }
          >
            Restart {and(groups)} now
          </Button>
          <Button size="sm" variant="secondary" onClick={done}>
            Wait for the next deploy
          </Button>
        </span>
      }
    />
  )
}

/* ── secrets and variables ──────────────────────────────────────────── */

type VarRow = {
  name: string
  type: 'secret' | 'variable' | 'builtin' | 'managed'
  status: 'set' | 'missing' | 'empty_default'
  value: string | null
  /** `undefined`: unknown (the worker's list, not iii cloud's). */
  applies?: string | null
  updated?: string | null
  required_by: string[]
  request?: SecretRequest
  resource?: Resource
}

/** iii cloud's variables (else the ones the worker last read), the agent's latest ask winning the words, then the managed resources. */
function varRows(s: Status, envId: string, cloud: CloudVariable[] | null, resources: Resource[]): VarRow[] {
  const latest = new Map<string, SecretRequest>()
  for (const r of s.secrets) if (r.environment_id === envId && !latest.has(r.name)) latest.set(r.name, r)
  const known = cloud !== null
  const list: CloudVariable[] =
    cloud ?? s.variables.map((v) => ({ ...v, value: null, applies: null, updated_at: null }))
  const rows: VarRow[] = list.map((v) => ({
    name: v.name,
    type: v.type,
    status: v.status,
    required_by: v.required_by,
    value: v.value,
    applies: known ? v.applies : undefined,
    updated: known ? v.updated_at : undefined,
    request: latest.get(v.name),
  }))
  for (const r of latest.values()) {
    if (!rows.some((x) => x.name === r.name)) {
      rows.push({ name: r.name, type: 'secret', status: 'missing', value: null, required_by: [], request: r })
    }
  }
  for (const r of resources) {
    if (r.mode !== 'managed' || !r.credentials) continue
    rows.push({
      name: r.connection?.map((x) => x.name).join(', ') || `${r.kind} ${r.alias}`,
      type: 'managed',
      status: 'set',
      value: null,
      updated: r.credentials.rotated_at ?? r.created_at,
      required_by: [],
      resource: r,
    })
  }
  return rows
}

const ASKED: Partial<Record<SecretRequest['state'], string>> = {
  waiting: 'asked by your agent',
  left_empty: 'left empty',
  failed: 'not saved',
}

function kindText(r: VarRow): string {
  if (r.type === 'managed') return 'managed by iii cloud'
  if (r.type === 'builtin') return 'set by iii cloud'
  const q = r.request
  const said = q?.state === 'noted' ? (q.mode === 'decide' ? 'your decision' : 'not needed') : q && ASKED[q.state]
  if (said) return `${r.type} · ${said}`
  if (r.status === 'empty_default') return 'optional · not set'
  return r.status === 'missing' ? `${r.type} · not set` : r.type
}

function takesText(r: VarRow): string {
  if (r.applies === undefined || r.status !== 'set' || r.type === 'managed' || r.type === 'builtin') return '—'
  if (r.applies === 'live') return 'live'
  const g = groupsOf(r.applies)
  return g.length ? `after ${and(g)} restart${g.length > 1 ? '' : 's'}` : 'next deploy'
}

const ADD = '+add'
const NAME = /^[A-Z_][A-Z0-9_]*$/

function Variables({
  c,
  vars,
  res,
  narrow,
  ask,
  read,
}: {
  c: Ctx
  vars: Cloud<{ variables: CloudVariable[] }>
  res: Cloud<{ resources: Resource[] }>
  narrow: boolean
  ask: (a: Ask) => void
  /** The names the values editor lists: they are its rows, not these. */
  read: Set<string>
}) {
  const [open, setOpen] = useState<string | null>(null)
  const rows = varRows(c.status, c.envId, vars.data?.variables ?? null, res.data?.resources ?? []).filter(
    (r) => !read.has(r.name),
  )
  const reads = vars.data?.variables?.find((v) => v.required_by.length > 0)
  const toggle = (name: string) => setOpen(open === name ? null : name)

  const action = (r: VarRow) => {
    if (r.type === 'builtin') return null
    if (r.type === 'managed') {
      const rotating = !!r.resource?.credentials?.rotating
      return (
        <Button
          size="sm"
          variant="secondary"
          disabled={rotating}
          onClick={() =>
            ask({
              title: `Rotate the credentials of ${r.name}?`,
              description: `iii cloud makes new credentials, gives them to every environment that uses it and redeploys the running ones. The old credentials keep working until those deploys are ready, at least 10 minutes.`,
              label: 'Rotate credentials',
              run: async () => {
                await cloudWrite(
                  c.host,
                  c.status,
                  c.envId,
                  'POST',
                  `/resources/${enc(r.resource?.id ?? '')}/rotate-credentials`,
                )
                res.refetch()
              },
            })
          }
        >
          {rotating ? 'Rotating…' : 'Rotate credentials'}
        </Button>
      )
    }
    const label = r.status !== 'set' ? 'Set' : r.type === 'secret' ? 'Replace' : 'Edit'
    return (
      <Button
        size="sm"
        variant="secondary"
        aria-label={`${label} ${r.name}`}
        aria-expanded={open === r.name}
        onClick={() => toggle(r.name)}
      >
        {label}
      </Button>
    )
  }

  const remove = (r: VarRow) =>
    r.status === 'set' ? (
      <Button
        variant="secondary"
        disabled={r.required_by.length > 0}
        onClick={() =>
          ask({
            title: `Delete ${r.name} from ${c.envName}?`,
            description: `${r.name} is removed from iii cloud for ${c.envName}. This can't be undone.`,
            label: 'Delete',
            tone: 'danger',
            run: async () => {
              await cloudWrite(c.host, c.status, c.envId, 'DELETE', `/variables/${enc(r.name)}?confirm=${enc(r.name)}`)
              setOpen(null)
              vars.refetch()
            },
          })
        }
      >
        Delete
      </Button>
    ) : null

  const form = (r: VarRow) =>
    r.type === 'secret' ? (
      <SecretEntry
        host={c.host}
        status={c.status}
        name={r.name}
        why={
          r.request?.description ||
          (r.required_by.length ? `Read by ${and(r.required_by)}.` : 'Set on the Settings page.')
        }
        request={r.request?.state === 'waiting' ? r.request : undefined}
        onSaved={vars.refetch}
        extra={
          <>
            <Button variant="secondary" onClick={() => setOpen(null)}>
              Close
            </Button>
            {remove(r)}
          </>
        }
      />
    ) : (
      <VariableForm
        c={c}
        name={r.name}
        initial={r.value ?? ''}
        onCancel={() => setOpen(null)}
        onSaved={() => {
          setOpen(null)
          vars.refetch()
        }}
        extra={remove(r)}
      />
    )

  return (
    <Panel aria-label="Secrets and variables">
      <PanelHeader className="iii-cloud-panel-head">
        <span className="iii-cloud-grow">Secrets and variables</span>
        <Button size="sm" variant="secondary" aria-expanded={open === ADD} onClick={() => toggle(ADD)}>
          Add
        </Button>
      </PanelHeader>
      {vars.error ? (
        <PanelBody>
          <Notice
            variant="warn"
            headline="Couldn't read the variables from iii cloud"
            detail={plainMessage(vars.error)}
            action={
              <Button size="sm" variant="secondary" onClick={vars.refetch}>
                Retry
              </Button>
            }
          />
        </PanelBody>
      ) : null}
      {open === ADD ? (
        <PanelBody>
          <AddForm
            c={c}
            onClose={() => setOpen(null)}
            onSaved={() => {
              setOpen(null)
              vars.refetch()
            }}
            onSecretSaved={vars.refetch}
          />
        </PanelBody>
      ) : null}
      {vars.loading && !vars.data && rows.length === 0 ? (
        <PanelBody>
          <Skeleton className="iii-cloud-set-skel" />
        </PanelBody>
      ) : rows.length === 0 ? (
        <PanelBody>
          {read.size ? (
            <EmptyState
              compact
              title="No other secrets or variables"
              description="The ones your app file reads are above."
            />
          ) : (
            <EmptyState
              compact
              title="No secrets or variables yet"
              description="Add one, or your agent asks you for the ones the app needs."
            />
          )}
        </PanelBody>
      ) : narrow ? (
        <List aria-label="Secrets and variables">
          {rows.map((r) => (
            <Fragment key={r.name}>
              <ListItem
                as="div"
                label={<Mono>{r.name}</Mono>}
                description={`${kindText(r)} · ${takesText(r)}`}
                trailing={action(r)}
              />
              {open === r.name ? <div className="iii-cloud-set-form">{form(r)}</div> : null}
            </Fragment>
          ))}
        </List>
      ) : (
        <TableViewport>
          <Table inset className="iii-cloud-table">
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Kind</TableHead>
                <TableHead>Takes effect</TableHead>
                <TableHead>Updated</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((r) => (
                <Fragment key={r.name}>
                  <TableRow>
                    <TableCell>
                      <Mono>{r.name}</Mono>
                    </TableCell>
                    <TableCell>{kindText(r)}</TableCell>
                    <TableCell>
                      {r.applies === 'live' ? <StatusChip tone="success">live</StatusChip> : takesText(r)}
                    </TableCell>
                    <TableCell>{r.updated ? ago(r.updated) : '—'}</TableCell>
                    <TableCell>{action(r)}</TableCell>
                  </TableRow>
                  {open === r.name ? (
                    <TableRow>
                      <TableCell colSpan={5}>{form(r)}</TableCell>
                    </TableRow>
                  ) : null}
                </Fragment>
              ))}
            </TableBody>
          </Table>
        </TableViewport>
      )}
      <PanelBody>
        <p className="iii-cloud-desc">
          Secret values are never shown here.
          {reads ? ` A name a worker needs can't be deleted: ${and(reads.required_by)} reads ${reads.name}.` : ''}
        </p>
      </PanelBody>
    </Panel>
  )
}

/** A variable's value, written as the person with `PUT /variables/{name}`. Never a secret: `api::write` refuses those. */
function VariableForm({
  c,
  name,
  initial,
  onCancel,
  onSaved,
  extra,
}: {
  c: Ctx
  name: string
  initial: string
  onCancel: () => void
  onSaved: () => void
  extra?: ReactNode
}) {
  const [value, setValue] = useState(initial)
  const w = useWrite()
  const id = useId()
  const save = () =>
    w.run('put', async () => {
      await cloudWrite(c.host, c.status, c.envId, 'PUT', `/variables/${enc(name)}`, { type: 'variable', value })
      onSaved()
    })
  return (
    <div className="iii-cloud-secret">
      <label className="iii-cloud-lbl" htmlFor={id}>
        <Mono>{name}</Mono>
      </label>
      <div className="iii-cloud-row">
        <div className="iii-cloud-grow">
          <Input id={id} value={value} onChange={setValue} autoComplete="off" />
        </div>
        <Button variant="primary" disabled={w.busy !== null || value === initial} onClick={() => void save()}>
          Save
        </Button>
        <Button variant="secondary" onClick={onCancel}>
          Cancel
        </Button>
        {extra}
      </div>
      <span className="iii-cloud-psmall">Not secret: everyone on the team can read a variable's value.</span>
      {w.error ? <ErrorPanel error={w.error} /> : null}
    </div>
  )
}

function AddForm({
  c,
  onClose,
  onSaved,
  onSecretSaved,
}: {
  c: Ctx
  onClose: () => void
  onSaved: () => void
  onSecretSaved: () => void
}) {
  const [name, setName] = useState('')
  const [kind, setKind] = useState<'variable' | 'secret'>('variable')
  const id = useId()
  const reserved = name.startsWith('III_')
  return (
    <div className="iii-cloud-set-body">
      <label className="iii-cloud-lbl" htmlFor={id}>
        Name
      </label>
      <div className="iii-cloud-row">
        <div className="iii-cloud-grow">
          <Input
            id={id}
            value={name}
            onChange={(v) => setName(v.toUpperCase().replace(/[^A-Z0-9_]/g, ''))}
            autoComplete="off"
          />
        </div>
        <SegmentedControl
          variant="radio"
          aria-label="Kind"
          value={kind}
          onChange={setKind}
          options={[
            { value: 'variable', label: 'Variable', icon: false },
            { value: 'secret', label: 'Secret', icon: false },
          ]}
        />
      </div>
      {reserved ? <span className="iii-cloud-psmall">Names that start with III_ are iii cloud's own.</span> : null}
      {!NAME.test(name) || reserved ? (
        <div className="iii-cloud-row">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
        </div>
      ) : kind === 'secret' ? (
        <SecretEntry
          key={name}
          host={c.host}
          status={c.status}
          name={name}
          why="Added on the Settings page."
          onSaved={onSecretSaved}
          extra={
            <Button variant="secondary" onClick={onClose}>
              Close
            </Button>
          }
        />
      ) : (
        <VariableForm key={name} c={c} name={name} initial="" onCancel={onClose} onSaved={onSaved} />
      )}
    </div>
  )
}

/* ── Mng11: check another compose file ──────────────────────────────── */

function fileLine(f: ComposeFile, current: string, env: string): string {
  return [
    f.path === current ? `what ${env} runs now` : null,
    f.workers != null ? `${f.workers} worker${f.workers === 1 ? '' : 's'}` : null,
    f.choosable ? null : ((f.issue && COMPOSE_ISSUE[f.issue]) ?? f.issue),
  ]
    .filter(Boolean)
    .join(' · ')
}

/**
 * The compose files of the check's copy of the code. Checking one never changes the environment: it
 * switches only when the person confirms a deploy of that check.
 */
function ChangeFile({ c, go, onClose }: { c: Ctx; go: (r: Route) => void; onClose: () => void }) {
  const files = c.status.compose_files ?? []
  const current = composeFile(c.status.project?.environment?.manifest)
  const checked = appFile(c.status).path
  const [pick, setPick] = useState(checked)
  const { call } = useActions(c.host)
  const ws = c.status.project?.workspace
  const ok = files.some((f) => f.path === pick && f.choosable) && pick !== checked
  const start = () => {
    // The check runs on; its stage shows it from the hub's `checking` count.
    void call('check', 'iii-cloud::check', { run: true, manifest: pick })
    onClose()
    go({ at: 'deploy', env: c.envId })
  }
  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent className="iii-cloud-cf">
        <DialogTitle>Change the compose file {c.envName} runs</DialogTitle>
        {ws ? (
          <DialogDescription>
            Files found in <Mono>{ws.path}</Mono> on <Mono>{ws.branch}</Mono>.
          </DialogDescription>
        ) : null}
        <List role="radiogroup" aria-label="Compose file">
          {files.map((f) => (
            <ListItem
              key={f.path}
              role="radio"
              aria-checked={pick === f.path}
              selected={pick === f.path}
              disabled={!f.choosable}
              leading={pick === f.path ? <CircleDot /> : <Circle />}
              label={<Mono>{f.path}</Mono>}
              description={fileLine(f, current, c.envName)}
              onClick={() => setPick(f.path)}
            />
          ))}
        </List>
        <Panel aria-label={`What changes on ${c.envName}`}>
          <PanelHeader className="iii-cloud-panel-head">What changes on {c.envName}</PanelHeader>
          <PanelBody className="iii-cloud-set-body">
            The check plans this file and shows every difference before you deploy.
          </PanelBody>
        </Panel>
        <div className="iii-cloud-cf-foot">
          <p className="iii-cloud-desc">
            {c.envName} keeps the current file until a deploy with the new one passes the check and you confirm it.
            Pushes until then still use the current file.
          </p>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!ok} onClick={start}>
            Check with this file
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

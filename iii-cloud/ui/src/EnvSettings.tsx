import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  ConfirmDialog,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  EmptyState,
  IconButton,
  Input,
  List,
  ListItem,
  Panel,
  SegmentedControl,
  ShadcnTextarea,
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
import { Ellipsis } from 'lucide-react'
import { Fragment, type ReactNode, useId, useState } from 'react'
import { cloudWrite, useCloud } from './api'
import { clock } from './asked'
import { useActions } from './hub'
import { plainMessage, plural, servingDeploy, valueMessage, when } from './model'
import { ErrorPanel, Mono, Notice } from './parts'
import type { EnvTabProps } from './route'
import { SecretEntry } from './SecretEntry'
import type { SecretRequest, Status, ValueItem, Values, ValuesSaved, WorkerError } from './types'
import { parseEnv, ReadOnly, useValues, type ValuesRead } from './ValuesEditor'

/* ── what iii cloud answers (cloud-v1.yaml: Variable) ── */

type CloudVariable = {
  name: string
  type: 'secret' | 'variable' | 'builtin'
  value: string | null
  status: 'set' | 'missing' | 'empty_default'
  required_by: string[]
  applies: string | null
  updated_at: string | null
}

type Ctx = { host: Host; status: Status; envId: string; envName: string }
type Ask = { title: string; description: string; label: string; tone?: 'danger'; run: () => Promise<void> }
type Kind = 'secret' | 'variable'

const and = (xs: string[]) => (xs.length < 2 ? (xs[0] ?? '') : `${xs.slice(0, -1).join(', ')} and ${xs.at(-1)}`)
const enc = encodeURIComponent

/** `after_restart:app,jobs` → the groups that read the value when they restart. */
const groupsOf = (applies: string | null | undefined) =>
  applies?.startsWith('after_restart:') ? applies.slice('after_restart:'.length).split(',').filter(Boolean) : []

/** One cloud write at a time: the busy key and the cloud's error, its 403 message kept. */
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

const restartKey = (envId: string, v: CloudVariable) => `restart:${envId}:${v.name}:${v.updated_at}`

/* ── the rows: every name once ──────────────────────────────────────── */

type Row = {
  name: string
  /** `managed`: iii cloud fills it in (a database URL, a built-in). */
  kind: Kind | 'managed'
  set: boolean
  /** The file defaults it, so the environment deploys without it. */
  optional: boolean
  /** A variable's value, never a secret's. */
  value: string | null
  updated: string | null
  /** `undefined`: not known (iii cloud's list was not read). */
  applies?: string | null
  required_by: string[]
  containers: string[]
  default: string | null
  request?: SecretRequest
}

/**
 * Every name once: the ones the compose file reads (file order), then the other secrets and variables of
 * the environment in iii cloud (else the ones the worker last read), then names only the agent asked for.
 */
function secretRows(s: Status, envId: string, values: Values | null, cloud: CloudVariable[] | null): Row[] {
  const latest = new Map<string, SecretRequest>()
  for (const r of s.secrets) if (r.environment_id === envId && !latest.has(r.name)) latest.set(r.name, r)
  const known = cloud !== null
  const list: CloudVariable[] =
    cloud ?? s.variables.map((v) => ({ ...v, value: null, applies: null, updated_at: null }))
  const byName = new Map(list.map((v) => [v.name, v]))
  const rows: Row[] = []
  const seen = new Set<string>()
  for (const i of values?.items ?? []) {
    seen.add(i.name)
    const c = byName.get(i.name)
    const set = c ? c.status === 'set' : i.in_cloud === 'set'
    rows.push({
      name: i.name,
      // A name typed from here starts as a secret; the row's menu makes it a variable.
      kind: i.database || c?.type === 'builtin' ? 'managed' : set ? (c?.type ?? i.type) : 'secret',
      set,
      optional: !set && i.need !== 'required',
      value: set ? (c?.value ?? i.value) : null,
      updated: c?.updated_at ?? i.updated_at,
      applies: known ? c?.applies : undefined,
      required_by: c?.required_by ?? [],
      containers: i.containers,
      default: i.default,
      request: latest.get(i.name),
    })
  }
  for (const v of list) {
    if (seen.has(v.name)) continue
    seen.add(v.name)
    rows.push({
      name: v.name,
      kind: v.type === 'builtin' ? 'managed' : v.status === 'set' ? v.type : 'secret',
      set: v.status === 'set',
      optional: v.status === 'empty_default',
      value: v.value,
      updated: v.updated_at,
      applies: known ? v.applies : undefined,
      required_by: v.required_by,
      containers: [],
      default: null,
      request: latest.get(v.name),
    })
  }
  for (const r of latest.values()) {
    if (seen.has(r.name)) continue
    rows.push({
      name: r.name,
      kind: 'secret',
      set: false,
      optional: false,
      value: null,
      updated: null,
      required_by: [],
      containers: [],
      default: null,
      request: r,
    })
  }
  return rows
}

const kindText = (r: Row) => (r.kind === 'managed' ? 'set by iii cloud' : !r.set && r.optional ? 'optional' : r.kind)

/** What the Value column says: the value or where it came from, or why there is none. */
function valueText(r: Row, env: string): ReactNode {
  if (r.kind === 'managed') return 'iii cloud makes the database and fills this in. Nothing to do.'
  if (r.set) {
    if (r.kind === 'secret') return `Set${r.updated ? ` · ${when(r.updated)}` : ''}`
    return r.value === '' ? 'Left empty' : r.value === null ? 'Set' : <Mono>{r.value}</Mono>
  }
  if (r.request?.state === 'waiting') return 'Asked by your agent: type it here.'
  if (r.optional)
    return `Not set. Your compose file gives it ${r.default ? `the default ${r.default}` : 'an empty default'}, so ${env} deploys without it.`
  return `Not set. ${r.containers.length ? `${and(r.containers)} reads it` : 'A worker needs it'}: the check asks for it too.`
}

/** When a saved value reaches the running machines. */
function takesText(r: Row): string {
  if (r.applies === undefined || !r.set || r.kind === 'managed') return '—'
  if (r.applies === 'live') return 'live'
  const g = groupsOf(r.applies)
  return g.length ? `after ${and(g)} restart${g.length > 1 ? '' : 's'}` : 'next deploy'
}

/* ── the page ───────────────────────────────────────────────────────── */

/**
 * An environment's Secrets and variables: the one place that lists every name the compose file reads. A secret's
 * value goes only through `iii-cloud::secret::provide` (`SecretEntry`), a variable's through `api::write`, a
 * pasted .env through `iii-cloud::values::save`.
 */
export function EnvSettings({ host, status, narrow, paneKey, envId }: EnvTabProps) {
  const vars = useCloud<{ variables: CloudVariable[] }>(host, status, envId, '/variables')
  const values = useValues(host, status, {})
  const [dismissed, setDismissed] = usePaneState<string[]>(`iii-cloud:settings-dismissed:${paneKey}`, [])
  const dismiss = (...keys: string[]) => setDismissed((d) => [...d, ...keys])
  const [confirm, setConfirm] = useState<Ask | null>(null)
  const [open, setOpen] = useState<{ name: string; kind: Kind } | null>(null)
  const [adding, setAdding] = useState(false)
  const [pasting, setPasting] = useState(false)
  const c: Ctx = { host, status, envId, envName: status.project?.environment?.name ?? 'this environment' }
  const v = values.data
  const rows = secretRows(status, envId, v, vars.data?.variables ?? null)
  const readOnly = !!v && v.save !== 'ok'
  const loading = (vars.loading && !vars.data) || (values.loading && !v)
  const optional = rows.filter((r) => !r.set && r.optional).length
  const missing = rows.filter((r) => !r.set && !r.optional && r.kind !== 'managed').length
  const reads = rows.find((r) => r.required_by.length > 0)

  const done = () => {
    setOpen(null)
    vars.refetch()
    values.refetch()
  }
  const action = (r: Row) => {
    if (r.kind === 'managed') return null
    const label = !r.set ? 'Set' : r.kind === 'secret' ? 'Replace' : 'Edit'
    return (
      <Button
        size="sm"
        variant="secondary"
        disabled={readOnly}
        aria-label={`${label} ${r.name}`}
        aria-expanded={open?.name === r.name}
        onClick={() => setOpen(open?.name === r.name ? null : { name: r.name, kind: r.kind as Kind })}
      >
        {label}
      </Button>
    )
  }
  /** The kind is the row's own; turning a secret into a variable (or back) takes the value again. */
  const kindCell = (r: Row) => (
    <span className="iii-cloud-kind">
      {kindText(r)}
      {r.kind === 'managed' || readOnly ? null : (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <IconButton label={`Change the kind of ${r.name}`} variant="ghost">
              <Ellipsis />
            </IconButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            <DropdownMenuItem
              onSelect={() => setOpen({ name: r.name, kind: r.kind === 'secret' ? 'variable' : 'secret' })}
            >
              {r.kind === 'secret' ? 'Make it a variable' : 'Make it a secret'}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </span>
  )
  const remove = (r: Row) =>
    r.set ? (
      <Button
        variant="secondary"
        disabled={r.required_by.length > 0}
        onClick={() =>
          setConfirm({
            title: `Delete ${r.name} from ${c.envName}?`,
            description: `${r.name} is removed from iii cloud for ${c.envName}. This can't be undone.`,
            label: 'Delete',
            tone: 'danger',
            run: async () => {
              await cloudWrite(host, status, envId, 'DELETE', `/variables/${enc(r.name)}?confirm=${enc(r.name)}`)
              done()
            },
          })
        }
      >
        Delete
      </Button>
    ) : null
  const form = (r: Row) =>
    open?.kind === 'secret' ? (
      <SecretEntry
        host={host}
        status={status}
        name={r.name}
        why={
          r.request?.description ||
          (r.required_by.length ? `Read by ${and(r.required_by)}.` : 'Set on the Secrets and variables page.')
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
        onSaved={done}
        extra={remove(r)}
      />
    )

  return (
    <div className="iii-cloud-set">
      <div className="iii-cloud-set-head">
        <div className="iii-cloud-rec-title">
          <h2 className="iii-cloud-h">Secrets and variables</h2>
          <p className="iii-cloud-desc">
            {v ? (
              <>
                <Mono>{v.manifest}</Mono> reads {plural(v.items.length, 'name')} in {c.envName}
              </>
            ) : (
              `${plural(rows.length, 'name')} in ${c.envName}`
            )}
            {missing ? ` · ${missing} not set` : ''}
            {optional ? ` · ${optional} optional ${optional === 1 ? "one isn't" : "ones aren't"} set` : ''}
          </p>
        </div>
        <span className="iii-cloud-row">
          <Button variant="secondary" disabled={!v || readOnly} onClick={() => setPasting(true)}>
            Paste a .env
          </Button>
          <Button variant="primary" disabled={readOnly} aria-expanded={adding} onClick={() => setAdding(!adding)}>
            Add
          </Button>
        </span>
      </div>
      {v && readOnly ? <ReadOnly host={host} status={status} values={v} /> : null}
      <Restart c={c} vars={vars.data?.variables ?? []} dismissed={dismissed} dismiss={dismiss} ask={setConfirm} />
      <Asked c={c} />
      {values.error ? (
        <Notice
          variant="warn"
          headline={`Couldn't read the names your compose file reads`}
          detail={plainMessage(values.error)}
          action={
            <Button size="sm" variant="secondary" onClick={values.refetch}>
              Retry
            </Button>
          }
        />
      ) : null}
      {vars.error ? (
        <Notice
          variant="warn"
          headline="Couldn't read the secrets and variables from iii cloud"
          detail={plainMessage(vars.error)}
          action={
            <Button size="sm" variant="secondary" onClick={vars.refetch}>
              Retry
            </Button>
          }
        />
      ) : null}
      {adding ? (
        <Panel className="iii-cloud-panel iii-cloud-set-add">
          <AddForm
            c={c}
            onClose={() => setAdding(false)}
            onSaved={() => {
              setAdding(false)
              done()
            }}
            onSecretSaved={vars.refetch}
          />
        </Panel>
      ) : null}
      <Panel className="iii-cloud-panel" role="region" aria-label="Secrets and variables">
        {loading && rows.length === 0 ? (
          <Skeleton className="iii-cloud-set-skel" />
        ) : rows.length === 0 ? (
          <EmptyState
            compact
            title="No secrets or variables yet"
            description="Add one, paste a .env, or let your agent ask you for the ones the app needs."
          />
        ) : narrow ? (
          <List aria-label="Names">
            {rows.map((r) => (
              <Fragment key={r.name}>
                <ListItem
                  as="div"
                  label={<Mono>{r.name}</Mono>}
                  description={
                    <>
                      {kindText(r)} · {valueText(r, c.envName)}
                      {r.set ? ` · ${takesText(r)}` : ''}
                    </>
                  }
                  trailing={action(r)}
                />
                {open?.name === r.name ? <div className="iii-cloud-set-form">{form(r)}</div> : null}
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
                  <TableHead>Value</TableHead>
                  <TableHead>Takes effect</TableHead>
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
                      <TableCell>{kindCell(r)}</TableCell>
                      <TableCell>
                        <span className="iii-cloud-desc">{valueText(r, c.envName)}</span>
                      </TableCell>
                      <TableCell>{takesText(r)}</TableCell>
                      <TableCell>{action(r)}</TableCell>
                    </TableRow>
                    {open?.name === r.name ? (
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
      </Panel>
      <p className="iii-cloud-desc iii-cloud-set-note">
        This is the only place that lists every name. A value you type, or one read from a .env file, is saved as a
        secret; a row's menu turns it into a variable. Secret values are never shown here.
        {reads ? ` A name a worker needs can't be deleted: ${and(reads.required_by)} reads ${reads.name}.` : ''} A value
        a deploy is missing shows up in its check, where you enter it.
      </p>
      {pasting && v ? (
        <PasteEnv host={host} values={v} read={values} onClose={() => setPasting(false)} onSaved={vars.refetch} />
      ) : null}
      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(o) => (o ? undefined : setConfirm(null))}
        title={confirm?.title ?? ''}
        description={confirm?.description}
        confirmLabel={confirm?.label}
        tone={confirm?.tone}
        onConfirm={() => confirm?.run()}
      />
    </div>
  )
}

/* ── what became of the agent's asks ────────────────────────────────── */

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

/* ── a value changed after the serving deployment started ───────────── */

function Restart({
  c,
  vars,
  dismissed,
  dismiss,
  ask,
}: {
  c: Ctx
  vars: CloudVariable[]
  dismissed: string[]
  dismiss: (...keys: string[]) => void
  ask: (a: Ask) => void
}) {
  const serving = servingDeploy(c.status, c.envId)
  const since = serving ? Date.parse(serving.created_at) : Number.NaN
  const changed = vars.filter(
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
  const at = changed
    .map((v) => v.updated_at as string)
    .sort()
    .at(-1) as string
  const done = () => dismiss(...changed.map((v) => restartKey(c.envId, v)))
  return (
    <Notice
      variant="warn"
      headline={`${and(names)} changed at ${clock(at)}`}
      detail={`The ${and(groups)} ${many ? 'groups keep' : 'group keeps'} the old ${names.length > 1 ? 'values' : 'value'} until ${many ? 'they restart' : 'it restarts'}. The next deploy restarts ${many ? 'them' : 'it'} too.`}
      action={
        <span className="iii-cloud-row">
          <Button
            size="sm"
            variant="primary"
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

/* ── forms ──────────────────────────────────────────────────────────── */

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

const NAME = /^[A-Z_][A-Z0-9_]*$/

/** Add: a name and its kind (a secret unless said otherwise), then the value. */
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
  const [kind, setKind] = useState<Kind>('secret')
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
            { value: 'secret', label: 'Secret', icon: false },
            { value: 'variable', label: 'Variable', icon: false },
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
          why="Added on the Secrets and variables page."
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

/**
 * Paste a .env: the names the compose file reads are saved as secrets through `iii-cloud::values::save` (the
 * row's menu makes one a variable afterwards). The pasted text lives in this dialog's state only and is gone
 * when it closes; a name that did not save is listed with why, and the rest are done.
 */
function PasteEnv({
  host,
  values: v,
  read,
  onClose,
  onSaved,
}: {
  host: Host
  values: Values
  read: ValuesRead
  onClose: () => void
  onSaved: () => void
}) {
  const [text, setText] = useState('')
  const [failed, setFailed] = useState<{ name: string; message: string }[]>([])
  const { busy, error, call } = useActions(host)
  const id = useId()
  const listed = new Map<string, ValueItem>(v.items.map((i) => [i.name, i]))
  const found = parseEnv(text)
  const mine = found.filter((e) => listed.has(e.name) && !listed.get(e.name)?.database)
  const other = found.filter((e) => !listed.has(e.name)).map((e) => e.name)
  const save = async () => {
    const out = await call<ValuesSaved>('save', 'iii-cloud::values::save', {
      environment_id: v.environment_id,
      manifest: v.manifest,
      items: mine.map((e) => ({ name: e.name, from: 'typed', type: 'secret', value: e.value })),
    })
    if (out === null) return
    onSaved()
    read.refetch()
    const bad = (out.results ?? []).filter((r) => !r.ok)
    if (!bad.length) return onClose()
    setFailed(
      bad.map((r) => ({
        name: r.name,
        message: r.error ? valueMessage(r.error, { env: v.environment, name: r.name }) : "iii cloud didn't take it.",
      })),
    )
    // What saved is done: only the names that did not stay in the box.
    setText((t) =>
      parseEnv(t)
        .filter((e) => bad.some((b) => b.name === e.name))
        .map((e) => `${e.name}=${e.value}`)
        .join('\n'),
    )
  }
  return (
    <Dialog open onOpenChange={(o) => (o ? undefined : onClose())}>
      <DialogContent>
        <DialogTitle>Paste a .env</DialogTitle>
        <DialogDescription>
          Names that <Mono>{v.manifest}</Mono> reads are saved to {v.environment} as secrets. Nothing else is kept, and
          the pasted text is gone when this closes.
        </DialogDescription>
        <label className="iii-cloud-lbl" htmlFor={id}>
          The lines of a .env file
        </label>
        <ShadcnTextarea
          id={id}
          className="iii-cloud-val-area"
          value={text}
          rows={8}
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => setText(e.target.value)}
        />
        {found.length ? (
          <p className="iii-cloud-desc">
            {mine.length
              ? `${plural(mine.length, 'name')} will be saved.`
              : 'None of these names is read by the compose file.'}
            {other.length ? ` Not read by the compose file, so skipped: ${other.join(', ')}.` : ''}
          </p>
        ) : null}
        {failed.map((f) => (
          <Notice key={f.name} variant="alert" headline={`${f.name} did not save`} detail={f.message} />
        ))}
        {error ? <ErrorPanel error={error} /> : null}
        <div className="iii-cloud-dlg-foot">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!mine.length || busy === 'save'} onClick={() => void save()}>
            {busy === 'save' ? 'Saving…' : mine.length ? `Save ${plural(mine.length, 'secret')}` : 'Save'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

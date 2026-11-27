// The secrets and variables the compose file reads, in a dialog (the check's and the deploy's): one row per name, where its value comes from,
// one Save through `iii-cloud::values::save`. Typed and pasted values live in this component's state only: never
// in pane state, the route, storage or a log. A row's value clears once it is saved, and all of them on unmount.

import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  Chip,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Input,
  SegmentedControl,
  Select,
  ShadcnTextarea,
  Skeleton,
} from '@iii-dev/console-ui'
import { errorCode, errorMessage } from '@iii-dev/console-ui/format'
import { CircleAlert } from 'lucide-react'
import { Fragment, type ReactNode, useCallback, useEffect, useId, useRef, useState } from 'react'
import { preselected } from './findings'
import { useActions } from './hub'
import { ago, envPageUrl, plainMessage, plural, valueMessage, workerError } from './model'
import { Dot, ErrorPanel, Mono, Notice, Spinner, StepHeader, StepLink } from './parts'
import type { SaveItem, Status, ValueItem, Values, ValuesSaved, ValuesSkipped, WorkerError } from './types'

/**
 * The lines of a .env file, as iii cloud's console reads them (`env-import.tsx`): `NAME=value`, `export NAME=value`,
 * a value in single or double quotes up to the first closing one (inside double quotes `\"` doesn't close it, and
 * reads as a quote, `\n` as a new line), a `#` comment on its own line or after a value. A quote that never closes
 * reads as unquoted text. The last line of a name wins.
 */
// ponytail: no raw multi-line quoted values
export function parseEnv(text: string): { name: string; value: string }[] {
  const found = new Map<string, string>()
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim()
    if (!line || line.startsWith('#')) continue
    const match = /^(?:export\s+)?([^=\s]+)\s*=\s*(.*)$/.exec(line)
    if (!match) continue
    const [, name, rest] = match
    let value = rest
    const quote = value[0]
    let end = quote === "'" ? value.indexOf("'", 1) : -1
    if (quote === '"')
      for (let i = 1; i < value.length; i++) {
        if (value[i] === '\\') i++
        else if (value[i] === '"') {
          end = i
          break
        }
      }
    if (end > 0) {
      value = value.slice(1, end)
      if (quote === '"') value = value.replaceAll('\\n', '\n').replaceAll('\\"', '"')
    } else {
      value = value.replace(/\s+#.*$/, '').trim()
    }
    found.delete(name)
    found.set(name, value)
  }
  return [...found].map(([name, value]) => ({ name, value }))
}

/** A name with no value in iii cloud the person is asked about (a default in the file is not asked). */
/** The local run has no value for it (`LOCAL_VARIABLE_UNSET`); never the database, the local run makes its own Postgres. */
export const unsetHere = (i: ValueItem) => i.here === 'unset' && !i.database
/** Set in iii cloud, but the run on this machine has no value: only this machine can fix it (an env file, or a skip). Never written to iii cloud from here. */
export const missingHere = (i: ValueItem) => i.in_cloud === 'set' && unsetHere(i)
/** The env files the run reads next to the compose file or in the folder, for an environment: `.env.<name>` first. */
export const runFiles = (env: string) => [
  ...(/^[a-z0-9-]+$/.test(env) && env !== 'example' ? [`.env.${env}`] : []),
  '.env.production',
  '.env.local',
  '.env',
]
export const notSet = (i: ValueItem) => (i.in_cloud !== 'set' && i.need !== 'has_default') || unsetHere(i)

export type ValuesRead = { data: Values | null; error: WorkerError | null; loading: boolean; refetch: () => void }
type Ask = { manifest?: string | null; from?: 'workspace' | 'checkout' }

/**
 * `iii-cloud::values` for one compose file in the selected environment, read again when the file or the environment
 * changes, when a check finishes and on `refetch`. `ask` null reads nothing. An answer to an older request, or for
 * an environment that is no longer the selected one, is dropped: another environment's values never show.
 */
export function useValues(host: Host, status: Status, ask: Ask | null): ValuesRead {
  const env = status.project?.environment?.id ?? ''
  const key = ask ? JSON.stringify([ask.manifest ?? null, ask.from ?? null, env]) : null
  const [state, setState] = useState<{ key: string | null; data: Values | null; error: WorkerError | null }>({
    key: null,
    data: null,
    error: null,
  })
  const [busy, setBusy] = useState(false)
  const [tick, setTick] = useState(0)
  const seq = useRef(0)
  const refetch = useCallback(() => setTick((t) => t + 1), [])
  const check = status.check ? `${status.check.id}:${status.check.run?.status ?? ''}` : ''
  useEffect(() => {
    if (key === null) return
    const mine = ++seq.current
    const [manifest, from, selected] = JSON.parse(key) as [string | null, string | null, string]
    setBusy(true)
    host.iii
      .call<Values>(
        'iii-cloud::values',
        { ...(manifest ? { manifest } : {}), ...(from ? { from } : {}) },
        {
          timeoutMs: 60_000,
        },
      )
      .then(
        (out) => {
          if (mine !== seq.current) return
          const ours = Array.isArray(out?.items) && (!selected || out.environment_id === selected)
          setState({ key, data: ours ? out : null, error: null })
          setBusy(false)
        },
        (err) => {
          if (mine !== seq.current) return
          setState((s) => ({ ...s, key, error: workerError(err, errorCode(err), errorMessage(err)) }))
          setBusy(false)
        },
      )
  }, [host, key, check, tick])
  const fresh = state.key === key
  return {
    data: fresh ? state.data : null,
    error: fresh ? state.error : null,
    loading: key !== null && (!fresh || busy),
    refetch,
  }
}

type Mode = 'step' | 'dialog'
type Kind = 'secret' | 'variable'
/** Where a row's value comes from: `env_file:<file>`, `copy:<environment id>` or one of the plain ones. */
type Source = string
type Pick = { source: Source; value?: string; multi?: boolean }
/** `note`: a saved value the local run's .env didn't get (LOCAL_NOT_SAVED); `file`: the source it failed with. */
type RowState =
  | { s: 'saving' }
  | { s: 'saved'; note?: WorkerError }
  | { s: 'error'; error?: WorkerError; file?: string }

export type ValuesEditorProps = {
  host: Host
  status: Status
  /** The compose file; none: the latest check's, else the environment's. */
  manifest?: string | null
  /** `checkout`: the compose file of the person's folder (Deploy step 1); default the agent's copy. */
  from?: 'workspace' | 'checkout'
  mode: Mode
  narrow: boolean
  /** After a save where every value was written. */
  onSaved?: (results: ValuesSaved['results']) => void
  /** step: "Check" and "Check without saving"; dialog: "Cancel". */
  onSkip?: () => void
  /** What follows a save, said on its button in a dialog ("and check again"). */
  after?: string
  /** Buttons off (a check is running). */
  disabled?: boolean
  /** A read the caller already holds. */
  read?: ValuesRead
  /** The dialog's description when `only` is given (the check's default says the local run needs them). */
  describe?: string
  /** Only these names (the check's "values the local run needs"); every name the file reads when none of them is listed. */
  only?: string[]
  /** step: the line under the step's title (its header holds the forward action: Save N values and check, or Check). */
  lead?: ReactNode
}

/** How the compose file points a database URL at the Postgres iii cloud makes (the `database` worker's binding). */
const DATABASE_REF = '$' + '{DATABASE_PRIMARY_URL}'

/** A name that wraps only after its underscores (`HARNESS_E2E_` / `OIDC_AUDIENCE`), never inside a word. */
const breakable = (name: string) => name.split(/(?<=_)/).flatMap((part, n) => (n ? [<wbr key={n} />, part] : [part]))

/** "required", "optional" or "default: development", an outlined tag. */
function Need({ item: i }: { item: ValueItem }) {
  return (
    <span className="iii-cloud-val-need">
      {i.need === 'has_default' ? (
        <>
          default: <Mono>{i.default ?? ''}</Mono>
        </>
      ) : (
        i.need
      )}
    </span>
  )
}

/** Where the name stands in iii cloud, or what just happened to it. */
function State({ item: i, saved }: { item: ValueItem; saved: boolean }) {
  if (i.skipped && !saved)
    return (
      <Chip tone="neutral" className="iii-cloud-val-state">
        Skipped
      </Chip>
    )
  const [tone, dot, label]: ['success' | 'warning' | 'danger', 'ok' | 'warn' | 'alert', string] = saved
    ? ['success', 'ok', 'Saved · just now']
    : missingHere(i)
      ? ['warning', 'warn', 'Set in iii cloud · missing on this machine']
      : unsetHere(i)
        ? ['warning', 'warn', i.asked ? 'Asked by your agent' : 'Not set']
        : i.in_cloud === 'set'
          ? ['success', 'ok', `Set in iii cloud${i.updated_at ? ` · ${ago(i.updated_at)}` : ''}`]
          : i.in_cloud === 'unknown'
            ? ['danger', 'alert', "iii cloud's value could not be read"]
            : ['warning', 'warn', i.asked ? 'Asked by your agent' : 'Not set']
  return (
    <Chip tone={tone} className="iii-cloud-val-state">
      <Dot tone={dot} />
      {label}
    </Chip>
  )
}

/** The row's choice before the person makes one: Keep a set name (unless the local run lacks it), else the first preselected source. */
function initial(i: ValueItem): Pick {
  if (i.skipped) return { source: 'skip' }
  if (missingHere(i)) return { source: 'none' }
  if (i.in_cloud === 'set' && !unsetHere(i)) return { source: 'keep' }
  if (i.database || i.invalid_name) return { source: 'none' }
  const s = preselected(i)[0]
  if (s?.source === 'env_file') return { source: `env_file:${s.file}` }
  if (s?.source === 'env_url') return { source: 'env_url' }
  return { source: 'typed', value: '' }
}

/** `skip`: "Skip for now" on a row the cloud or the local run needs. */
function options(i: ValueItem, skip: boolean): { value: string; label: string }[] {
  // iii cloud already has it: the only choice from here is to skip it on this machine.
  if (missingHere(i)) return skip ? [{ value: 'skip', label: 'Skip for now' }] : []
  const out: { value: string; label: string }[] = []
  if (i.in_cloud === 'set') out.push({ value: 'keep', label: 'Keep' })
  out.push({ value: 'typed', label: i.in_cloud === 'set' ? 'Replace' : 'Type a value' })
  for (const s of i.suggestions) {
    if (s.source === 'env_file')
      out.push({ value: `env_file:${s.file}`, label: `From ${s.file}${s.local ? ' (points at localhost)' : ''}` })
    else if (s.source === 'env_url') out.push({ value: 'env_url', label: "This environment's address" })
    else out.push({ value: `copy:${s.environment_id}`, label: `Copy from ${s.environment}` })
  }
  // The worker accepts it for any name; a name with a default keeps its default.
  if (i.need !== 'has_default') out.push({ value: 'leave_empty', label: 'Leave empty' })
  if (skip && (notSet(i) || i.skipped)) out.push({ value: 'skip', label: 'Skip for now' })
  return out
}

const fileOf = (p: Pick) => (p.source.startsWith('env_file:') ? p.source.slice('env_file:'.length) : undefined)

/** "7 from .env.test · 6 typed · 1 address · 3 copied · 1 left empty". */
function summary(items: SaveItem[]): string {
  const files = new Map<string, number>()
  let typed = 0
  let url = 0
  let copied = 0
  let empty = 0
  for (const x of items) {
    if (x.from === 'env_file') files.set(x.file, (files.get(x.file) ?? 0) + 1)
    else if (x.from === 'copy') copied++
    else if (x.from === 'typed') typed++
    else if (x.from === 'env_url') url++
    else empty++
  }
  return [
    ...[...files].map(([f, n]) => `${n} from ${f}`),
    typed ? `${typed} typed` : null,
    url ? plural(url, 'address', 'addresses') : null,
    copied ? `${copied} copied` : null,
    empty ? `${empty} left empty` : null,
  ]
    .filter(Boolean)
    .join(' · ')
}

/** The names a compose file reads, each with where its value comes from, and one Save for all of them. */
export function ValuesEditor(props: ValuesEditorProps) {
  const own = useValues(props.host, props.status, props.read ? null : { manifest: props.manifest, from: props.from })
  const read = props.read ?? own
  // Choices belong to one environment: another one starts with none, so a typed value never follows a switch.
  return <Editor key={read.data?.environment_id ?? ''} {...props} read={read} />
}

function Editor(props: ValuesEditorProps & { read: ValuesRead }) {
  const { host, status, mode, narrow, read } = props
  const v = read.data
  const { busy, error, call } = useActions(host)
  const [picks, setPicks] = useState<Record<string, Pick>>({})
  const [kinds, setKinds] = useState<Record<string, Kind>>({})
  const [outside, setOutside] = useState<Record<string, boolean>>({})
  const [rows, setRows] = useState<Record<string, RowState>>({})
  const [filter, setFilter] = useState<'unset' | 'all' | null>(null)
  const [showAll, setShowAll] = useState(false)
  const [paste, setPaste] = useState<string | null>(null)
  // What the last Apply filled, and the pasted names the file doesn't read (names only; the text is gone).
  const [report, setReport] = useState<{ filled: number; other: string[] } | null>(null)
  const pasteId = useId()

  const env = v?.environment ?? status.project?.environment?.name ?? 'this environment'
  const every = v?.items ?? []
  const some = props.only ? every.filter((i) => props.only?.includes(i.name)) : []
  const items = some.length ? some : every
  const readOnly = !!v && v.save !== 'ok'
  const unset = items.filter(notSet).length
  const saved = (i: ValueItem) => rows[i.name]?.s === 'saved'
  const pickOf = (i: ValueItem): Pick =>
    picks[i.name] ?? (saved(i) ? { source: 'keep' } : outside[i.name] ? { source: 'typed', value: '' } : initial(i))
  const kindOf = (i: ValueItem): Kind => kinds[i.name] ?? (outside[i.name] ? 'secret' : i.type)
  const toSave = (i: ValueItem): SaveItem | null => {
    // iii cloud already has it: nothing about it is ever sent to `values::save` from here.
    if (missingHere(i)) return null
    const p = pickOf(i)
    const file = fileOf(p)
    if (p.source === 'typed') return p.value ? { name: i.name, from: 'typed', type: kindOf(i), value: p.value } : null
    if (p.source === 'env_url') return { name: i.name, from: 'env_url' }
    if (p.source === 'leave_empty') return { name: i.name, from: 'leave_empty' }
    if (file) return { name: i.name, from: 'env_file', type: kindOf(i), file }
    if (p.source.startsWith('copy:')) return { name: i.name, from: 'copy', environment_id: p.source.slice(5) }
    return null
  }
  // A name the file no longer reads is never saved: only the listed rows count.
  const chosen = readOnly ? [] : items.map(toSave).filter((x): x is SaveItem => x !== null)
  const n = chosen.length
  // Skipped for now (`values::skip`) on Save, after the values; a recorded skip is not sent again.
  const canSkip = !readOnly
  const skips = canSkip ? items.filter((i) => !i.skipped && pickOf(i).source === 'skip').map((i) => i.name) : []
  const k = skips.length
  // "Skip the rest": every "Not set" row still without a choice (and with a picker).
  const rest = canSkip
    ? items.filter(
        (i) => notSet(i) && !i.database && !i.invalid_name && !i.skipped && !toSave(i) && pickOf(i).source !== 'skip',
      )
    : []
  const skipHere = async (name: string) => {
    if (!v) return
    const out = await call<ValuesSkipped>('skip', 'iii-cloud::values::skip', {
      environment_id: v.environment_id,
      names: [name],
      skip: true,
    })
    if (out !== null) read.refetch()
  }
  const set = (name: string, p: Pick) => setPicks((x) => ({ ...x, [name]: p }))

  const save = async () => {
    if (!v || (!n && !k)) return
    // The values first, then the skips; the step goes on only when both went through.
    const results = n ? await saveValues(v) : []
    if (results === null) return
    let ok = !n || (results.length > 0 && results.every((x) => x.ok))
    if (ok && k) {
      const out = await call<ValuesSkipped>('skip', 'iii-cloud::values::skip', {
        environment_id: v.environment_id,
        names: skips,
        skip: true,
      })
      if (out === null) ok = false
      else setPicks((p) => Object.fromEntries(Object.entries(p).filter(([name]) => !skips.includes(name))))
    }
    read.refetch()
    if (ok) props.onSaved?.(results)
  }
  const undo = async (name: string) => {
    if (!v) return
    const out = await call<ValuesSkipped>('skip', 'iii-cloud::values::skip', {
      environment_id: v.environment_id,
      names: [name],
      skip: false,
    })
    if (out === null) return
    setPicks((p) => Object.fromEntries(Object.entries(p).filter(([x]) => x !== name)))
    read.refetch()
  }

  /** The chosen values, through `values::save`: each row says how it went; null when the whole call was refused. */
  const saveValues = async (v: Values): Promise<ValuesSaved['results'] | null> => {
    const names = chosen.map((x) => x.name)
    const files = new Map(chosen.flatMap((x) => (x.from === 'env_file' ? [[x.name, x.file] as const] : [])))
    setRows((r) => ({ ...r, ...Object.fromEntries(names.map((name) => [name, { s: 'saving' as const }])) }))
    const out = await call<ValuesSaved>('save', 'iii-cloud::values::save', {
      environment_id: v.environment_id,
      manifest: v.manifest,
      items: chosen,
    })
    if (out === null) {
      // The whole call was refused (the error says why): every choice stays for the next Save.
      setRows((r) => Object.fromEntries(Object.entries(r).filter(([name]) => !names.includes(name))))
      return null
    }
    const results = out.results ?? []
    const ok = new Set(results.filter((x) => x.ok).map((x) => x.name))
    setRows((r) => {
      const next = { ...r }
      for (const x of results)
        next[x.name] = x.ok ? { s: 'saved', note: x.error } : { s: 'error', error: x.error, file: files.get(x.name) }
      return next
    })
    setPicks((p) => Object.fromEntries(Object.entries(p).filter(([name]) => !ok.has(name))))
    setReport(null)
    return results
  }

  const apply = () => {
    if (paste === null) return
    const listed = new Map(items.map((i) => [i.name, i]))
    const other: string[] = []
    const next: Record<string, Pick> = {}
    for (const e of parseEnv(paste)) {
      const i = listed.get(e.name)
      if (!i) other.push(e.name)
      else if (!i.database || outside[i.name])
        next[i.name] = { source: 'typed', value: e.value, multi: /\n/.test(e.value) }
    }
    setPicks((p) => ({ ...p, ...next }))
    setPaste(null)
    setReport({ filled: Object.keys(next).length, other })
  }

  // Read-only lists every name (nothing to choose); else what isn't set comes first.
  const shown = readOnly ? 'all' : (filter ?? (unset ? 'unset' : 'all'))
  const allSet = !!v && unset === 0
  const listed = allSet && !showAll ? [] : shown === 'unset' ? items.filter((i) => notSet(i) || rows[i.name]) : items
  const first = (i: ValueItem) => i.containers[0] ?? ''
  const groups = new Set(listed.map(first)).size > 1
  const order = groups ? [...new Set(listed.map(first))] : ['']

  const saving = busy === 'save' || busy === 'skip'
  const off = !!props.disabled || saving
  // "Save 12 values", "Save 3 · skip 7", "Skip 7".
  const what = n && k ? `Save ${n} · skip ${k}` : n ? `Save ${plural(n, 'value')}` : k ? `Skip ${k}` : 'Save'
  const label = (more?: string) => (saving ? 'Saving…' : [what, more].filter(Boolean).join(' '))
  const primary = (more?: string) => (
    <Button variant="primary" disabled={off || (!n && !k)} onClick={() => void save()}>
      {saving ? <Spinner /> : null}
      {label(more)}
    </Button>
  )
  const check = (
    <Button variant={n ? 'secondary' : 'primary'} disabled={off} onClick={props.onSkip}>
      {n ? 'Check without saving' : 'Check'}
    </Button>
  )
  // The step's actions are in its header (P1); a dialog's in the footer, the primary last.
  const acts =
    mode === 'step' ? null : mode === 'dialog' ? (
      <>
        <Button variant="secondary" onClick={props.onSkip}>
          {readOnly ? 'Close' : 'Cancel'}
        </Button>
        {readOnly ? null : primary(props.after)}
      </>
    ) : n ? (
      primary()
    ) : null
  const sum =
    n || k
      ? `${[n ? summary(chosen) : null, k ? `${k} skipped` : null].filter(Boolean).join(' · ')}${report ? ' · the pasted text is gone' : ''}`
      : readOnly
        ? v?.save === 'role'
          ? 'Ask an Owner or Admin, or continue to the check.'
          : "Continue still works: the check lists what's missing."
        : allSet
          ? 'Nothing to save. The check reads these.'
          : v
            ? 'Nothing chosen yet'
            : null

  const row = (i: ValueItem) => (
    <ValueRow
      key={i.name}
      item={i}
      env={env}
      readOnly={readOnly || allSet}
      pick={pickOf(i)}
      kind={kindOf(i)}
      state={rows[i.name]}
      outside={!!outside[i.name]}
      also={groups ? i.containers.slice(1) : []}
      onPick={(p) => set(i.name, p)}
      onKind={(k) => setKinds((x) => ({ ...x, [i.name]: k }))}
      onOutside={() => setOutside((x) => ({ ...x, [i.name]: true }))}
      skip={canSkip}
      onUndo={readOnly ? null : () => void undo(i.name)}
      onSkipHere={readOnly ? null : () => void skipHere(i.name)}
    />
  )
  const list = listed.length ? (
    <div className="iii-cloud-val-list">
      {order.map((c) => {
        const of = listed.filter((i) => !groups || first(i) === c)
        return (
          <section key={c || 'all'} className="iii-cloud-val-group" aria-label={c || undefined}>
            {groups ? (
              <h3 className="iii-cloud-val-ghead">
                {c || 'Asked by your agent'} <span className="iii-cloud-faint">· {of.length}</span>
              </h3>
            ) : null}
            <ul className="iii-cloud-val-rows">{of.map(row)}</ul>
          </section>
        )
      })}
    </div>
  ) : null

  return (
    <div
      className="iii-cloud-val"
      data-mode={mode}
      data-narrow={narrow ? '' : undefined}
      data-allset={allSet ? '' : undefined}
    >
      {mode === 'step' ? (
        <StepHeader
          title="Give the secrets and variables your app reads"
          sub={props.lead}
          back={
            n || k ? (
              <StepLink disabled={off} onClick={() => props.onSkip?.()}>
                Check without saving
              </StepLink>
            ) : null
          }
          forward={n || k ? primary('and check') : check}
        />
      ) : null}
      {mode === 'dialog' ? (
        <div className="iii-cloud-val-dhead">
          <DialogTitle>Secrets and variables for {env}</DialogTitle>
          {v ? (
            <DialogDescription>
              {some.length ? (
                (props.describe ??
                `The local run needs ${some.length === 1 ? 'this name' : `these ${some.length} names`} on this machine.`)
              ) : (
                <>
                  <Mono>{v.manifest}</Mono> reads {plural(items.length, 'name')};{' '}
                  {unset ? `${unset} ${unset === 1 ? "isn't" : "aren't"} set.` : 'all are set.'}
                </>
              )}
            </DialogDescription>
          ) : null}
        </div>
      ) : null}
      {read.error ? (
        <Notice
          variant="warn"
          headline="Couldn't read the names your compose file reads"
          detail={plainMessage(read.error)}
          action={
            <Button size="sm" variant="secondary" onClick={read.refetch}>
              Retry
            </Button>
          }
        />
      ) : !v ? (
        read.loading ? (
          <Skeleton className="iii-cloud-val-skel" />
        ) : null
      ) : (
        <>
          {readOnly ? <ReadOnly host={host} status={status} values={v} /> : null}
          {allSet ? (
            <>
              <p className="iii-cloud-val-lead">
                {items.length ? (
                  <>
                    All {plural(items.length, 'value')} <Mono>{v.manifest}</Mono> reads are set in {env}.
                  </>
                ) : (
                  <>
                    <Mono>{v.manifest}</Mono> reads no names.
                  </>
                )}
              </p>
              {items.length ? (
                <div className="iii-cloud-val-all">
                  <button
                    type="button"
                    className="iii-cloud-val-fold"
                    aria-expanded={showAll}
                    onClick={() => setShowAll(!showAll)}
                  >
                    Show all {items.length}
                  </button>
                  {list}
                </div>
              ) : null}
            </>
          ) : readOnly ? null : (
            <div className="iii-cloud-val-bar">
              {mode === 'dialog' ? null : (
                <span className="iii-cloud-val-count">
                  <Mono>{v.manifest}</Mono> reads {plural(items.length, 'name')}
                  {v.cloud === 'unreadable'
                    ? " · couldn't read iii cloud"
                    : unset
                      ? ` · ${unset} not set in ${env}`
                      : ''}
                </span>
              )}
              {unset ? (
                <SegmentedControl
                  variant="radio"
                  aria-label="Which names"
                  className="iii-cloud-val-filter"
                  itemClassName="iii-cloud-val-filter-item"
                  value={shown}
                  onChange={setFilter}
                  options={[
                    {
                      value: 'unset',
                      label: (
                        <>
                          Not set <span className="iii-cloud-val-n">{unset}</span>
                        </>
                      ),
                      icon: false,
                    },
                    {
                      value: 'all',
                      label: (
                        <>
                          All <span className="iii-cloud-val-n">{items.length}</span>
                        </>
                      ),
                      icon: false,
                    },
                  ]}
                />
              ) : null}
              {rest.length ? (
                <button
                  type="button"
                  className="iii-cloud-val-link iii-cloud-val-rest"
                  onClick={() =>
                    setPicks((p) => ({ ...p, ...Object.fromEntries(rest.map((i) => [i.name, { source: 'skip' }])) }))
                  }
                >
                  Skip the rest
                </button>
              ) : null}
              {paste !== null ? null : (
                <Button size="sm" variant="secondary" className="iii-cloud-val-paste-btn" onClick={() => setPaste('')}>
                  Paste a .env
                </Button>
              )}
            </div>
          )}
          {paste !== null ? (
            <div className="iii-cloud-val-paste">
              <label className="iii-cloud-lbl" htmlFor={pasteId}>
                The lines of a .env file
              </label>
              <ShadcnTextarea
                id={pasteId}
                className="iii-cloud-val-area"
                value={paste}
                rows={5}
                spellCheck={false}
                autoComplete="off"
                onChange={(e) => setPaste(e.target.value)}
              />
              <span className="iii-cloud-row">
                <Button size="sm" variant="primary" disabled={!paste.trim()} onClick={apply}>
                  Apply
                </Button>
                <Button size="sm" variant="secondary" onClick={() => setPaste(null)}>
                  Cancel
                </Button>
                <span className="iii-cloud-psmall">
                  Fills the names <Mono>{v.manifest}</Mono> reads. Nothing is saved before you Save.
                </span>
              </span>
            </div>
          ) : null}
          {report ? (
            <Notice
              variant="success"
              headline={
                <>
                  Filled {report.filled} of {items.length}.
                  {report.other.length ? (
                    <>
                      {' '}
                      Not read by <Mono>{v.manifest}</Mono>:{' '}
                      {report.other.map((name, i) => (
                        <Fragment key={name}>
                          {i ? ', ' : null}
                          <Mono>{name}</Mono>
                        </Fragment>
                      ))}
                      .
                    </>
                  ) : null}
                </>
              }
            />
          ) : null}
          {allSet ? null : list}
        </>
      )}
      {error ? <ErrorPanel error={error} /> : null}
      {acts || sum ? (
        <div className="iii-cloud-val-foot">
          <span className="iii-cloud-val-sum">{sum}</span>
          {acts ? <span className="iii-cloud-val-acts">{acts}</span> : null}
        </div>
      ) : null}
    </div>
  )
}

/** An API key, or production without Owner or Admin: the list only, and where to set the values instead. */
export function ReadOnly({ host, status, values }: { host: Host; status: Status; values: Values }) {
  const { busy, call } = useActions(host)
  const url = envPageUrl(status, 'config')
  if (values.save === 'role')
    return <Notice variant="info" headline="Changing production's secrets and variables takes Owner or Admin." />
  return (
    <Notice
      variant="info"
      headline="An API key can't write secrets and variables. Set them in iii cloud's console, or sign in with the browser."
      detail={
        <span className="iii-cloud-row iii-cloud-val-roacts">
          {url ? (
            <a className="iii-cloud-val-link" href={url} target="_blank" rel="noreferrer">
              Open {values.environment}'s values in iii cloud
            </a>
          ) : null}
          <Button
            size="sm"
            variant="secondary"
            disabled={busy === 'connect'}
            onClick={() => void call('connect', 'iii-cloud::connect', { method: 'browser' })}
          >
            Sign in with your browser
          </Button>
        </span>
      }
    />
  )
}

/** The Secret/Variable switch; a kind the source fixes is a faint word. */
function KindOf({
  item: i,
  pick: p,
  kind,
  saved,
  onKind,
}: {
  item: ValueItem
  pick: Pick
  kind: Kind
  saved: boolean
  onKind: (k: Kind) => void
}) {
  if (p.source === 'skip') return null
  if (p.source === 'env_url' || p.source === 'leave_empty') return <span className="iii-cloud-val-fixed">variable</span>
  if (p.source.startsWith('copy:')) {
    const from = i.suggestions.find((s) => s.source === 'copy' && `copy:${s.environment_id}` === p.source)
    return <span className="iii-cloud-val-fixed">as in {from?.source === 'copy' ? from.environment : 'the other'}</span>
  }
  if (p.source === 'keep' || saved) return <span className="iii-cloud-val-fixed">{i.type}</span>
  return (
    <SegmentedControl
      variant="radio"
      aria-label={`Kind of ${i.name}`}
      className="iii-cloud-val-kind"
      value={kind}
      onChange={onKind}
      options={[
        { value: 'secret', label: 'Secret', icon: false },
        { value: 'variable', label: 'Variable', icon: false },
      ]}
    />
  )
}

/** One name: what it is and its state, where its value comes from, the field for it, and its kind. */
function ValueRow({
  item: i,
  env,
  readOnly,
  pick: p,
  kind,
  state,
  outside,
  also,
  onPick,
  onKind,
  onOutside,
  skip,
  onUndo,
  onSkipHere,
}: {
  item: ValueItem
  env: string
  readOnly: boolean
  pick: Pick
  kind: Kind
  state: RowState | undefined
  outside: boolean
  also: string[]
  onPick: (p: Pick) => void
  onKind: (k: Kind) => void
  onOutside: () => void
  /** "Skip for now" in the picker. */
  skip: boolean
  /** Undo a recorded skip (null: read-only). */
  onUndo: (() => void) | null
  /** "Skip it here" on a name set in iii cloud and missing on this machine (null: read-only). */
  onSkipHere: (() => void) | null
}) {
  const url = i.suggestions.find((s) => s.source === 'env_url')
  const file = fileOf(p)
  // A secret's multi-line value is shown only while the person asks (never by default).
  const [show, setShow] = useState(false)
  const value = i.in_cloud === 'set' && i.type === 'variable' && i.value != null ? i.value : null
  const shownValue = value === '' ? 'Left empty' : value
  let field: ReactNode = null
  if (p.source === 'typed') {
    const label = `Value of ${i.name}`
    const secret = kind === 'secret'
    const toggle = (
      <button type="button" className="iii-cloud-val-link" onClick={() => onPick({ ...p, multi: !p.multi })}>
        {p.multi ? 'One line' : 'Multi-line'}
      </button>
    )
    field = p.multi ? (
      <span className="iii-cloud-val-multi">
        <ShadcnTextarea
          aria-label={label}
          className="iii-cloud-val-area"
          // A secret stays masked until the person shows it (a text area has no password type).
          data-masked={secret && !show ? '' : undefined}
          value={p.value ?? ''}
          rows={3}
          wrap="off"
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => onPick({ ...p, value: e.target.value })}
        />
        <span className="iii-cloud-val-note">
          {secret ? (
            <button type="button" className="iii-cloud-val-link" aria-pressed={show} onClick={() => setShow(!show)}>
              {show ? 'Hide' : 'Show'}
            </button>
          ) : null}
          {toggle}
        </span>
      </span>
    ) : (
      <span className="iii-cloud-val-one">
        <Input
          aria-label={label}
          type={secret ? 'password' : 'text'}
          placeholder={secret ? 'Paste the secret' : 'Value'}
          autoComplete="off"
          spellCheck={false}
          value={p.value ?? ''}
          onChange={(v) => onPick({ ...p, value: v })}
        />
        {secret ? toggle : null}
      </span>
    )
  } else if (file) {
    field = <span className="iii-cloud-val-note">read from {file} when you save</span>
  } else if (p.source === 'env_url' && url?.source === 'env_url') {
    field = (
      <span className="iii-cloud-val-url">
        <Mono>{url.url}</Mono>
      </span>
    )
  } else if (p.source.startsWith('copy:')) {
    const from = i.suggestions.find((s) => s.source === 'copy' && `copy:${s.environment_id}` === p.source)
    field = (
      <span className="iii-cloud-val-note">
        {from?.source === 'copy' ? from.environment : 'the other environment'}'s value, copied inside iii cloud
      </span>
    )
  } else if (p.source === 'skip') {
    field = <span className="iii-cloud-val-note">{i.skipped ? 'skipped for now' : 'skipped when you save'}</span>
  } else if (p.source === 'leave_empty') {
    field = <span className="iii-cloud-val-note">saved as an empty variable</span>
  } else if (p.source === 'keep') {
    field =
      shownValue !== null ? (
        <span className="iii-cloud-val-url">{value === '' ? shownValue : <Mono>{shownValue}</Mono>}</span>
      ) : (
        <button type="button" className="iii-cloud-val-link" onClick={() => onPick({ source: 'typed', value: '' })}>
          Replace
        </button>
      )
  }
  const notes = [
    i.asked ? `Your agent: “${i.asked.description}”` : null,
    also.length ? `Also read by ${also.join(', ')}.` : null,
    // A copy stays in iii cloud: the local run's .env gets only what is typed or read from a file.
    p.source.startsWith('copy:') && i.need === 'required' ? 'iii cloud only; the local run needs it typed.' : null,
    // P3: what the local run has for it (saving a typed or file value writes the run's .env).
    unsetHere(i) && state?.s !== 'saved' ? 'needed for the local run' : null,
    i.here === 'from_file' && i.here_file ? `the local run takes it from ${i.here_file}` : null,
    state?.s === 'saved' && state.note ? valueMessage(state.note, { env, name: i.name }) : null,
  ].filter((x): x is string => !!x)
  return (
    <li className="iii-cloud-val-row" data-state={state?.s} data-name={i.name} data-ro={readOnly ? '' : undefined}>
      <span className="iii-cloud-val-name">
        <span className="iii-cloud-val-key">
          <Mono>{breakable(i.name)}</Mono>
        </span>
        <span className="iii-cloud-val-tags">
          <Need item={i} />
          <State item={i} saved={state?.s === 'saved'} />
          {i.skipped && onUndo && state?.s !== 'saved' ? (
            <button type="button" className="iii-cloud-val-link" onClick={onUndo}>
              Undo
            </button>
          ) : null}
        </span>
      </span>
      {readOnly ? (
        shownValue !== null ? (
          <span className="iii-cloud-val-wide iii-cloud-val-url">
            {value === '' ? shownValue : <Mono>{shownValue}</Mono>}
          </span>
        ) : null
      ) : missingHere(i) && !i.skipped ? (
        // iii cloud has it: no Replace, no typing, no empty value, no source. What fixes it is on this machine.
        <span className="iii-cloud-val-wide iii-cloud-val-note">
          Add it to an env file of your folder (the run reads {runFiles(env).join(', ')}), or skip it here to run it
          with an empty value.{' '}
          {onSkipHere ? (
            <button type="button" className="iii-cloud-val-link" onClick={onSkipHere}>
              Skip it here
            </button>
          ) : null}
        </span>
      ) : i.invalid_name ? (
        // iii cloud can't store this name: nothing to pick, nothing to type; why, said faint.
        <span className="iii-cloud-val-wide iii-cloud-val-note">{i.invalid_name}</span>
      ) : i.database && !outside ? (
        <span className="iii-cloud-val-wide iii-cloud-val-db">
          iii cloud makes this database. Your agent points <Mono>{i.name}</Mono> at <Mono>{DATABASE_REF}</Mono>.{' '}
          <button type="button" className="iii-cloud-val-link" onClick={onOutside}>
            It's outside iii cloud
          </button>
        </span>
      ) : (
        <>
          <span className="iii-cloud-val-pick">
            <Select
              className="iii-cloud-val-select"
              aria-label={`Where ${i.name} comes from`}
              value={p.source}
              options={options(i, skip)}
              onChange={(source: string) => onPick(source === 'typed' ? { source, value: '' } : { source })}
            />
          </span>
          <span className="iii-cloud-val-field">{field}</span>
          <span className="iii-cloud-val-kindcell">
            <KindOf item={i} pick={p} kind={kind} saved={state?.s === 'saved'} onKind={onKind} />
          </span>
        </>
      )}
      {notes.length || state?.s === 'saving' || state?.s === 'error' ? (
        <span className="iii-cloud-val-extra">
          {notes.map((t) => (
            <span key={t} className="iii-cloud-val-note">
              {t}
            </span>
          ))}
          {state?.s === 'saving' ? (
            <span className="iii-cloud-val-note">
              <Spinner />
              Saving…
            </span>
          ) : null}
          {state?.s === 'error' ? (
            <span className="iii-cloud-val-err" role="alert">
              <CircleAlert aria-hidden />
              <span>
                <b>Not saved.</b>{' '}
                {state.error
                  ? valueMessage(state.error, { env, name: i.name, file: state.file })
                  : "iii cloud didn't take it."}{' '}
                It stays chosen for the next Save.
              </span>
            </span>
          ) : null}
        </span>
      ) : null}
    </li>
  )
}

/** The editor in a dialog (Deploy step 1, the check): "Secrets and variables for test", the file's names, Save or Cancel. */
export function ValuesDialog({
  onClose,
  onSaved,
  ...p
}: Omit<ValuesEditorProps, 'mode' | 'onSkip'> & { onClose: () => void }) {
  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent showCloseButton={false} className="iii-cloud-vdialog" data-narrow={p.narrow ? '' : undefined}>
        <ValuesEditor
          {...p}
          mode="dialog"
          onSkip={onClose}
          onSaved={(results) => {
            onClose()
            onSaved?.(results)
          }}
        />
      </DialogContent>
    </Dialog>
  )
}

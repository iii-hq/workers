import {
  Badge,
  Button,
  CodeEditor,
  type CodeEditorHandle,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Input,
  type LiveAnnouncement,
  LiveRegion,
  Select,
  StatusDot,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { FileUp } from 'lucide-react'
import { Fragment, useEffect, useId, useMemo, useRef, useState } from 'react'
import { coded } from './AgentAccessView'
import type { Api } from './api'
import {
  crossRowBlockers,
  finalEntry,
  type ImportResult,
  type ImportRow,
  importEffects,
  importRow,
  importSummary,
  parseImport,
  type RowOutcome,
  renameRow,
  rowBlockers,
  scrub,
  startValues,
} from './importer'
import { middleTruncate } from './presentation'
import { asksGate, blockSentence, effectSentences, type SaveDeps, saveBefore, saveEntry } from './save'
import type { Server, ValidateResponse } from './types'

/** A file past this is not read: a client's own config is kilobytes, so it is something else. */
const MAX_FILE_BYTES = 20 * 1024 * 1024
const TARGET_CHARS = 28
const BADGE = {
  new: ['New', 'default'],
  replaces: ['Replaces', 'warn'],
  skipped: ['Skipped', 'default'],
} as const

type Values = Record<string, string>
type Stored = [Record<string, unknown>, Server[]]
/** The last check of the current inputs: validation, and the approval-gate count of each name a save would ask. */
interface Check {
  key: string
  reply?: ValidateResponse
  blocks?: Record<string, number>
  error?: string
}

const servers = (n: number) => `${n} server${n === 1 ? '' : 's'}`

/** Import MCP servers: a pasted config or a chosen file becomes rows to review, then each row's secrets go to the
 * vault, its env references are shared and its entry is saved. No pasted value is ever shown or kept after. */
export function ImportDialog({
  api,
  deps,
  existing,
  initialText,
  open,
  onClose,
  setDirty,
}: {
  api: Api
  deps: SaveDeps
  existing: string[]
  initialText?: string
  open: boolean
  /** `finished` once an import ran, whatever each row's outcome. */
  onClose: (finished: boolean) => void
  setDirty: (dirty: boolean) => void
}) {
  const [text, setText] = useState('')
  const [fileName, setFileName] = useState<string | null>(null)
  const [fileProblem, setFileProblem] = useState<string | null>(null)
  const [result, setResult] = useState<ImportResult | null>(null)
  const [rows, setRows] = useState<ImportRow[]>([])
  const [skip, setSkip] = useState<boolean[]>([])
  const [values, setValues] = useState<Values[]>([])
  const [check, setCheck] = useState<Check | null>(null)
  const [retries, setRetries] = useState(0)
  const [phase, setPhase] = useState<'edit' | 'importing' | 'done'>('edit')
  const [progress, setProgress] = useState(0)
  const [outcomes, setOutcomes] = useState<RowOutcome[]>([])
  const [failure, setFailure] = useState<string | null>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const fileInput = useRef<HTMLInputElement | null>(null)
  const editor = useRef<CodeEditorHandle | null>(null)
  const wantsFocus = useRef(false)
  const seq = useRef(0)
  // Read once per opening for the effects list; Import reads both again for each row's save.
  const stored = useRef<Promise<Stored> | null>(null)
  const counts = useRef(new Map<string, Promise<number>>())
  const id = useId()

  const load = (next: string, file: string | null = null) => {
    const parsed = next.trim() === '' ? null : parseImport(next, existing)
    setText(next)
    setFileName(file)
    setFileProblem(null)
    setResult(parsed)
    setRows(parsed?.rows ?? [])
    setSkip(parsed?.rows.map(() => false) ?? [])
    setValues(parsed?.rows.map(startValues) ?? [])
    setCheck(null)
    setFailure(null)
  }
  const reset = () => {
    load('')
    setPhase('edit')
    setOutcomes([])
    setAnnouncement(null)
    stored.current = null
    counts.current.clear()
  }
  useEffect(() => {
    if (!open) return
    wantsFocus.current = true
    load(initialText ?? '')
  }, [open, initialText])
  useEffect(() => {
    if (open) setDirty(phase === 'edit' && (text !== '' || fileName !== null))
  }, [open, phase, text, fileName, setDirty])

  const close = () => {
    if (phase === 'importing') return
    const finished = phase === 'done'
    reset()
    setDirty(false)
    onClose(finished)
  }

  // Every pasted secret and every typed value: no message reaches the screen with one in it.
  const hidden = useMemo(
    () => [...rows.flatMap((r) => r.secrets.map((s) => s.value)), ...values.flatMap((v) => Object.values(v))],
    [rows, values],
  )
  const say = (message: string) => scrub(message, hidden)
  const effective = rows.map((r, i): ImportRow => (skip[i] ? { ...r, status: 'skipped' } : r))
  const cross = crossRowBlockers(effective, values)
  const blockers = effective.map((r, i) => [...rowBlockers(r, values[i] ?? {}), ...cross[i]])
  const targets = effective.flatMap((r, i) => (r.status === 'skipped' ? [] : [i]))
  const ready = targets.length > 0 && targets.every((i) => blockers[i].length === 0)
  const finals = ready ? targets.map((i) => finalEntry(effective[i], values[i] ?? {})) : []
  // The candidate map, references only: a secret's position holds `secret://NAME`.
  const key = ready
    ? JSON.stringify(Object.fromEntries(targets.map((i, n) => [effective[i].name, finals[n].entry])))
    : ''
  const current = check !== null && check.key === key ? check : null
  const reply = current?.reply
  const clean = ready && reply !== undefined && targets.every((i) => reply.servers[effective[i].name]?.ok === true)

  // `retries` is a dependency only so Retry runs a failed check again.
  useEffect(() => {
    if (!open || phase !== 'edit' || key === '') return
    const mine = ++seq.current
    const candidate = JSON.parse(key) as Record<string, Record<string, unknown>>
    void (async () => {
      try {
        const answer = await api.validate(candidate)
        stored.current ??= Promise.all([
          api.readConfig().then((c) => c.mcpServers),
          api.listServers().then((l) => l.servers),
        ])
        const [config, listed] = await stored.current
        const blocks: Record<string, number> = {}
        for (const [name, entry] of Object.entries(candidate)) {
          const v = answer.servers[name]
          if (!v?.ok || !asksGate(saveBefore(false, name, config, listed), v.fingerprint, entry)) continue
          if (!counts.current.has(name)) counts.current.set(name, deps.blockCount(name))
          blocks[name] = await (counts.current.get(name) as Promise<number>)
        }
        if (mine === seq.current) setCheck({ key, reply: answer, blocks })
      } catch (err) {
        stored.current = null
        counts.current.clear()
        if (mine === seq.current) setCheck({ key, error: errorMessage(err) })
      }
    })()
  }, [open, phase, key, retries, api, deps])

  const effects =
    clean && current?.blocks
      ? importEffects(
          targets.map((i, n) => {
            const name = effective[i].name
            const blocks = current.blocks?.[name] ?? 0
            return {
              name,
              store: finals[n].toStore.map((s) => s.name),
              share: finals[n].toShare,
              lines: [...effectSentences(reply.servers[name].effects), ...(blocks > 0 ? [blockSentence(blocks)] : [])],
            }
          }),
        )
      : []

  const chooseFile = (file: File | undefined) => {
    if (!file) return
    if (file.size > MAX_FILE_BYTES) {
      load('', file.name)
      setFileProblem('File is too large')
      return
    }
    file.text().then(
      (content) => load(content, file.name),
      () => {
        load('', file.name)
        setFileProblem('Could not read this file')
      },
    )
  }

  const run = async () => {
    if (!clean || phase !== 'edit') return
    const plan = targets.map((i) => [effective[i], values[i] ?? {}] as const)
    setPhase('importing')
    setProgress(1)
    setFailure(null)
    try {
      const [{ mcpServers }, { servers: listed }] = await Promise.all([api.readConfig(), api.listServers()])
      const save: Parameters<typeof importRow>[0]['save'] = (name, entry) =>
        saveEntry({ ...deps, confirm: async () => true }, name, entry, saveBefore(false, name, mcpServers, listed))
      const out: RowOutcome[] = []
      for (const [n, [row, typed]] of plan.entries()) {
        setProgress(n + 1)
        out.push(await importRow({ storeSecret: api.storeSecret, shareEnv: api.shareEnv, save }, row, typed))
      }
      const said = out.map((o) => ({ ...o, message: say(o.message) }))
      // One render: the paste, the rows and the typed values go before the outcomes show.
      load('')
      setOutcomes(said)
      setPhase('done')
      setDirty(false)
      setAnnouncement((last) => ({ seq: (last?.seq ?? 0) + 1, text: importSummary(said), urgency: 'polite' }))
    } catch (err) {
      setPhase('edit')
      setFailure(say(errorMessage(err)))
    }
  }

  const editRow = (i: number, change: (row: ImportRow) => ImportRow) =>
    setRows((all) => all.map((r, j) => (j === i ? change(r) : r)))
  const editing = phase === 'edit'
  const label =
    phase === 'done'
      ? 'Done'
      : phase === 'importing'
        ? `Importing ${progress} of ${targets.length}…`
        : targets.length === 0
          ? 'Import servers'
          : `Import ${servers(targets.length)}`

  const renderRow = (row: ImportRow, i: number) => {
    const shown = effective[i]
    const [badge, tone] = BADGE[shown.status]
    const blocker = blockers[i][0]
    const errors = current?.reply?.servers[row.name]?.errors ?? []
    const more = shown.status !== 'skipped' && (row.needs.length > 0 || blocker !== undefined || errors.length > 0)
    return (
      <Fragment key={i}>
        <TableRow className="mcp-ui-import-row" data-more={more}>
          <TableCell>
            <Input
              className="mcp-ui-import-mono"
              aria-label="Server name"
              value={row.name}
              disabled={!editing || row.status === 'skipped'}
              onChange={(next) => editRow(i, (r) => renameRow(r, next, existing))}
            />
            {row.originalName !== row.name ? (
              <p className="mcp-ui-import-quiet">
                <code>{row.originalName}</code>
              </p>
            ) : null}
          </TableCell>
          <TableCell>
            <span className="mcp-ui-import-target">
              {row.transport ? `${row.transport} · ${middleTruncate(row.target, TARGET_CHARS)}` : '—'}
            </span>
          </TableCell>
          <TableCell>
            <div className="mcp-ui-import-status">
              <Badge variant={tone}>{badge}</Badge>
              {row.status !== 'skipped' ? (
                <Select
                  aria-label={`Import or skip ${row.name || 'this server'}`}
                  value={skip[i] ? 'skip' : 'import'}
                  options={[
                    { value: 'import', label: row.status === 'replaces' ? 'Replace' : 'Import' },
                    { value: 'skip', label: 'Skip' },
                  ]}
                  disabled={!editing}
                  onChange={(next) => setSkip((all) => all.map((s, j) => (j === i ? next === 'skip' : s)))}
                />
              ) : null}
            </div>
            {[row.skipReason, ...row.notes].filter(Boolean).map((note) => (
              <p key={note} className="mcp-ui-import-quiet">
                {coded(say(note as string))}
              </p>
            ))}
          </TableCell>
          <TableCell>
            {row.secrets.length === 0 ? <span className="mcp-ui-import-quiet">None</span> : null}
            {row.secrets.map((s, j) => (
              <div key={s.path} className="mcp-ui-import-secret">
                <span>{s.masked}</span>
                <Input
                  className="mcp-ui-import-mono"
                  aria-label={`Vault name for ${s.path}`}
                  value={s.vaultName}
                  disabled={!editing || shown.status === 'skipped'}
                  onChange={(next) =>
                    editRow(i, (r) => ({
                      ...r,
                      secrets: r.secrets.map((t, k) => (k === j ? { ...t, vaultName: next.trim() } : t)),
                    }))
                  }
                />
              </div>
            ))}
          </TableCell>
          <TableCell>
            {row.ignored.length === 0 ? <span className="mcp-ui-import-quiet">None</span> : null}
            {row.ignored.map(({ field, note }) => (
              <p key={field} className="mcp-ui-import-ignored">
                <code>{field}</code>
                {note ? ` (${note})` : null}
              </p>
            ))}
          </TableCell>
        </TableRow>
        {more ? (
          <TableRow className="mcp-ui-import-more">
            <TableCell colSpan={5}>
              {row.needs.length > 0 ? (
                // biome-ignore lint/a11y/useSemanticElements: a fieldset's own box breaks the grid; the role names it
                <div
                  className="mcp-ui-import-needs"
                  role="group"
                  aria-label={`Values for ${row.name || 'this server'}`}
                >
                  {row.needs.map((need, j) => (
                    <div key={need.path} className="mcp-ui-import-need">
                      <label htmlFor={`${id}-${i}-${j}`}>{need.label}</label>
                      <Input
                        id={`${id}-${i}-${j}`}
                        className={need.password ? undefined : 'mcp-ui-import-mono'}
                        type={need.password ? 'password' : 'text'}
                        autoComplete="off"
                        spellCheck={false}
                        value={values[i]?.[need.path] ?? ''}
                        disabled={!editing}
                        onChange={(next) =>
                          setValues((all) => all.map((v, k) => (k === i ? { ...v, [need.path]: next } : v)))
                        }
                      />
                    </div>
                  ))}
                </div>
              ) : null}
              {blocker !== undefined ? <p className="mcp-ui-import-block">{coded(say(blocker))}</p> : null}
              {errors.length > 0 ? (
                <ul className="mcp-ui-issues">
                  {errors.map((issue, k) => (
                    <li key={k}>
                      <span>
                        {issue.path ? (
                          <>
                            <code>{issue.path}</code>:{' '}
                          </>
                        ) : null}
                        {coded(say(issue.message))}
                      </span>
                    </li>
                  ))}
                </ul>
              ) : null}
            </TableCell>
          </TableRow>
        ) : null}
      </Fragment>
    )
  }

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? undefined : close())}>
      <DialogContent
        className="mcp-ui-import"
        showCloseButton={phase !== 'importing'}
        aria-busy={phase === 'importing' || undefined}
        // The editor mounts late and hidden, so its autoFocus misses and focus falls back to the dialog: until the
        // person acts, hand it on.
        onFocus={(e) => {
          if (wantsFocus.current && e.target === e.currentTarget) editor.current?.focus()
        }}
        onPointerDownCapture={() => {
          wantsFocus.current = false
        }}
        onKeyDownCapture={() => {
          wantsFocus.current = false
        }}
      >
        <div className="mcp-ui-import-body">
          <DialogTitle className="mcp-ui-import-title">Import MCP servers</DialogTitle>
          <DialogDescription>
            Paste a config from another client, or choose its file. Nothing runs until you import.
          </DialogDescription>
          {phase === 'done' ? (
            <div className="mcp-ui-import-outcomes">
              {outcomes.map((o, i) =>
                o.imported ? (
                  <p key={i} className="mcp-ui-import-outcome">
                    <StatusDot tone="ok" />
                    <span>
                      <code>{o.name}</code>: Imported
                    </span>
                  </p>
                ) : (
                  <StatusPanel key={i} variant="alert" headline={o.name} detail={o.message} />
                ),
              )}
            </div>
          ) : (
            <>
              {fileName === null ? (
                <CodeEditor
                  ref={editor}
                  className="mcp-ui-import-editor"
                  language="json"
                  aria-label="Config to import"
                  lineNumbers
                  fill
                  autoFocus
                  readOnly={!editing}
                  value={text}
                  onChange={(next) => load(next)}
                />
              ) : (
                <p className="mcp-ui-import-file">
                  <code>{fileName}</code>
                  {fileProblem === null ? <span> · {servers(rows.length)} found</span> : null}
                </p>
              )}
              <div className="mcp-ui-import-bar">
                <Button variant="secondary" size="sm" disabled={!editing} onClick={() => fileInput.current?.click()}>
                  <FileUp size={16} aria-hidden />
                  Choose file
                </Button>
                {fileName !== null ? (
                  <Button variant="ghost" size="sm" disabled={!editing} onClick={() => load('')}>
                    Paste instead
                  </Button>
                ) : null}
                <input
                  ref={fileInput}
                  className="mcp-ui-file-input"
                  type="file"
                  tabIndex={-1}
                  aria-hidden="true"
                  accept="application/json,.json"
                  onChange={(event) => {
                    chooseFile(event.currentTarget.files?.[0])
                    event.currentTarget.value = ''
                  }}
                />
                {result ? <Badge className="mcp-ui-import-source">{result.sourceLabel}</Badge> : null}
              </div>
              {fileProblem ? <StatusPanel variant="warn" headline={fileProblem} /> : null}
              {result?.error ? <StatusPanel variant="warn" headline={result.error} /> : null}
              {rows.length > 0 ? (
                <TableViewport className="mcp-ui-import-table">
                  <Table density="compact">
                    <TableHeader>
                      <TableRow>
                        <TableHead>Server</TableHead>
                        <TableHead>Connection</TableHead>
                        <TableHead>Status</TableHead>
                        <TableHead>Secrets</TableHead>
                        <TableHead>Ignored</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>{rows.map(renderRow)}</TableBody>
                  </Table>
                </TableViewport>
              ) : null}
              {current?.error ? (
                <StatusPanel
                  variant="alert"
                  headline="Could not check this import"
                  detail={say(current.error)}
                  action={
                    <Button variant="secondary" size="sm" onClick={() => setRetries((n) => n + 1)}>
                      Retry
                    </Button>
                  }
                />
              ) : null}
              {effects.length > 0 ? (
                // biome-ignore lint/a11y/useSemanticElements: a list of effects, not form fields
                <div className="mcp-ui-import-effects" role="group" aria-labelledby={`${id}-effects`}>
                  <p id={`${id}-effects`}>This import will</p>
                  <ul>
                    {effects.map(({ server, text: line }, i) => (
                      <li key={i}>
                        {server ? <code>{server}</code> : null}
                        <span>{say(line)}</span>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
              {failure ? (
                <StatusPanel variant="alert" role="alert" headline="Could not import" detail={failure} />
              ) : null}
            </>
          )}
        </div>
        <div className="mcp-ui-import-foot">
          {phase !== 'done' ? (
            <Button variant="secondary" size="sm" disabled={!editing} onClick={close}>
              Cancel
            </Button>
          ) : null}
          <Button
            variant="primary"
            size="sm"
            disabled={phase === 'importing' || (phase === 'edit' && !clean)}
            onClick={phase === 'done' ? close : () => void run()}
          >
            {label}
          </Button>
        </div>
        <LiveRegion announcement={announcement} />
      </DialogContent>
    </Dialog>
  )
}

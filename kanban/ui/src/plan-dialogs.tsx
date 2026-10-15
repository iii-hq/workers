import {
  Badge,
  Breadcrumb,
  Button,
  Checkbox,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  type Host,
  IconButton,
  Input,
  type PageRenderProps,
  Panel,
  PanelHeader,
  SegmentedControl,
  Selector,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableFooter,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  Toolbar,
  uiClasses,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useDebounce } from '@iii-dev/console-ui/hooks'
import { ArrowLeft, Bell, Check, Flag, Folder, Info, RotateCcw, Tag, TriangleAlert } from 'lucide-react'
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from 'react'
import {
  asciiLower,
  columnSummary,
  formatTarget,
  isIsoDay,
  isNextByDate,
  labelCounts,
  labelFamily,
  nameList,
  nextRelease,
  notDone,
  plural,
  resolveEntity,
  shipTargets,
  suggestName,
  ticketsWithLabel,
  willAdd,
} from './plan-rules'
import {
  type AgentProfile,
  announcePlanChange,
  api,
  type BoardData,
  type Column,
  type ColumnCounts,
  type ConvertResult,
  type GlyphTone,
  PLAN_CHANGED_EVENT,
  type ProjectView,
  type ReleaseView,
  StatusGlyph,
  subscribeChanges,
  type TicketSummary,
} from './shared'

/* ── data ─────────────────────────────────────────────────────────────── */

/**
 * The board as the plan pages read it: columns, live tickets, releases and projects in one call. Every ticket
 * change (any pane, any agent) refetches it once the burst settles; `reload(true)` refetches without a spinner.
 */
export function usePlanBoard(host: Host) {
  const [data, setData] = useState<BoardData | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const seq = useRef(0)
  // A quiet reload can supersede the first load: until a board has loaded, its failure must still show.
  const loaded = useRef(false)

  const reload = useCallback(
    async (quiet = false) => {
      const token = ++seq.current
      if (!quiet) {
        setLoading(true)
        setError(null)
      }
      try {
        const board = await api.board(host.iii)
        if (token !== seq.current) return
        setData(board)
        loaded.current = true
        setError(null)
      } catch (caught) {
        if (token !== seq.current) return
        if (!quiet || !loaded.current) setError(errorMessage(caught))
      } finally {
        if (token === seq.current) setLoading(false)
      }
    },
    [host],
  )

  useEffect(() => {
    void reload()
  }, [reload])

  // Another pane changed a project or a release (those writes fire no ticket event), or another tab did: read the
  // board again, quietly, when this tab comes back into view or gets the focus.
  useEffect(() => {
    const onPlanChange = () => void reload(true)
    const onVisible = () => {
      if (document.visibilityState === 'visible') void reload(true)
    }
    window.addEventListener(PLAN_CHANGED_EVENT, onPlanChange)
    window.addEventListener('focus', onPlanChange)
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      window.removeEventListener(PLAN_CHANGED_EVENT, onPlanChange)
      window.removeEventListener('focus', onPlanChange)
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [reload])

  useEffect(() => {
    let timer: number | undefined
    const off = subscribeChanges(host.iii, () => {
      window.clearTimeout(timer)
      timer = window.setTimeout(() => void reload(true), 300)
    })
    return () => {
      window.clearTimeout(timer)
      off()
    }
  }, [host, reload])

  return { data, loading, error, reload }
}

export function useAgents(host: Host): AgentProfile[] {
  const [agents, setAgents] = useState<AgentProfile[]>([])
  useEffect(() => {
    let cancelled = false
    void api
      .agents(host.iii)
      .then((result) => {
        if (!cancelled) setAgents(result.agents ?? [])
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [host])
  return agents
}

/** Lead choices: every agent profile, and the person at the console. */
export function agentOptions(agents: AgentProfile[], current?: string | null) {
  const options = [
    ...agents.map((agent) => ({ value: agent.id, label: agent.name, description: 'Agent profile' })),
    { value: 'user', label: 'User', description: 'Person' },
  ]
  // A lead whose profile is gone still shows as what it is.
  if (current && !options.some((option) => option.value === current))
    options.push({ value: current, label: current, description: '' })
  return options
}

export const TONES: GlyphTone[] = ['blue', 'purple', 'teal', 'green', 'amber', 'rose']

export const titleCase = (text: string) => text.charAt(0).toUpperCase() + text.slice(1)

/* ── small shared pieces ──────────────────────────────────────────────── */

/** One 16px folder in the project's identity tone (`ink-ghost` without one). */
export function ProjectGlyph({ color }: { color: GlyphTone | null }) {
  return (
    <span aria-hidden="true" className={`${uiClasses.treeItemIcon} kanban-plan-glyph`} data-color={color ?? undefined}>
      <Folder size={16} />
    </span>
  )
}

/** A derived status: the word in ink, the tone on the dot. */
export function StatusBadge({
  tone,
  children,
  title,
}: {
  tone: 'ink' | 'ghost' | 'ok'
  children: ReactNode
  title?: string
}) {
  return (
    <Badge className="kanban-plan-badge" title={title}>
      <span aria-hidden="true" className="kanban-plan-dot" data-tone={tone} />
      {children}
    </Badge>
  )
}

export function ProgressBar({ done, total, label }: { done: number; total: number; label?: string }) {
  const pct = total > 0 ? Math.round((done / total) * 100) : 0
  return (
    <span aria-label={label ?? `${done} of ${total} tickets done`} className="kanban-plan-bar" role="img">
      <span style={{ width: `${pct}%` }} />
    </span>
  )
}

/** The column counts of a release or a project, one tile per board column. */
export function ColumnTiles({
  columns,
  counts,
  extra,
}: {
  columns: Column[]
  counts: ColumnCounts
  extra?: ReactNode
}) {
  return (
    <div aria-label="Tickets by status" className="kanban-plan-tiles" role="group">
      {columns.map((column) => (
        <div className="kanban-plan-tile" key={column.id}>
          <span className="kanban-plan-tile__label">
            <StatusGlyph columns={columns} status={column.id} />
            {column.label}
          </span>
          <span className="kanban-plan-tile__n">{counts[column.id] ?? 0}</span>
        </div>
      ))}
      {extra}
    </div>
  )
}

/** The top of a narrow pane that shows one record: the way back, then where you are (`Releases / Phase 2`). */
export function DrillBar({ root, name, onBack }: { root: string; name: string; onBack: () => void }) {
  return (
    <Toolbar aria-label={`${root} navigation`} className="kanban-plan-drill">
      <IconButton
        className="kanban-plan-touch"
        label={`Back to the ${root.toLowerCase()}`}
        onClick={onBack}
        variant="ghost"
      >
        <ArrowLeft size={16} />
      </IconButton>
      <Breadcrumb items={[{ label: root, onClick: onBack }, { label: name }]} />
    </Toolbar>
  )
}

/**
 * Edit one field in place: the caller draws the value and a pencil, and swaps in `editor` while `editing`.
 * Enter or leaving the field saves a single line; a multi-line field has Save and Cancel (⌘↵ saves) so a stray
 * click never ships a half-written paragraph. Escape cancels. A refused save (a taken name) stays in the field.
 */
export function useInlineEdit({
  label,
  value,
  onSave,
  multiline = false,
  mono = false,
  placeholder,
  validate,
  emptyAllowed = true,
}: {
  label: string
  value: string
  onSave: (next: string) => Promise<void>
  multiline?: boolean
  mono?: boolean
  placeholder?: string
  validate?: (next: string) => string | null
  emptyAllowed?: boolean
}) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const busyRef = useRef(false)
  const skipBlur = useRef(false)
  const errorId = useId()

  const start = useCallback(() => {
    setDraft(value)
    setError(null)
    skipBlur.current = false
    setEditing(true)
  }, [value])

  const cancel = () => {
    skipBlur.current = true
    setEditing(false)
    setError(null)
  }

  const commit = async () => {
    if (busyRef.current) return
    const next = draft.trim()
    if (next === value.trim()) {
      setEditing(false)
      return
    }
    const problem =
      !emptyAllowed && next === '' ? `${titleCase(label)} is required` : next === '' ? null : (validate?.(next) ?? null)
    if (problem) {
      setError(problem)
      return
    }
    busyRef.current = true
    setBusy(true)
    setError(null)
    try {
      await onSave(next)
      setEditing(false)
    } catch (caught) {
      setError(errorMessage(caught))
    } finally {
      busyRef.current = false
      setBusy(false)
    }
  }

  const common = {
    'aria-describedby': error ? errorId : undefined,
    'aria-invalid': error ? (true as const) : undefined,
    'aria-label': titleCase(label),
    autoFocus: true,
    placeholder,
  }

  const editor = editing ? (
    <div className="kanban-plan-edit">
      {multiline ? (
        <textarea
          {...common}
          className="kanban-field__textarea"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault()
              event.stopPropagation()
              cancel()
            } else if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
              event.preventDefault()
              void commit()
            }
          }}
          rows={4}
          value={draft}
        />
      ) : (
        <Input
          {...common}
          className={mono ? 'kanban-plan-mono' : undefined}
          onBlur={() => {
            if (!skipBlur.current) void commit()
          }}
          onChange={setDraft}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault()
              void commit()
            } else if (event.key === 'Escape') {
              event.preventDefault()
              event.stopPropagation()
              cancel()
            }
          }}
          value={draft}
        />
      )}
      {multiline ? (
        <div className="kanban-plan-edit__foot">
          <Button className="kanban-plan-touch" onClick={cancel} size="sm" variant="ghost">
            Cancel
          </Button>
          <Button className="kanban-plan-touch" disabled={busy} onClick={() => void commit()} size="sm" variant="pill">
            Save
          </Button>
        </div>
      ) : null}
      {error ? (
        <p className="kanban-field__error" id={errorId} role="alert">
          {error}
        </p>
      ) : null}
    </div>
  ) : null

  return { editing, start, editor }
}

/** What `host.panels.open({ pageId, context })` asked of a page: an entity (`id`, by id or name) and/or an `action`. */
function panelRequest(panelContext: PageRenderProps['panelContext']): { id?: string; action?: string } {
  const context = panelContext?.context
  if (!context || typeof context !== 'object' || Array.isArray(context)) return {}
  const { id, action } = context as Record<string, unknown>
  return {
    id: typeof id === 'string' && id.trim() ? id : undefined,
    action: typeof action === 'string' ? action : undefined,
  }
}

const handledRequests = new Set<number>()

/**
 * The request a page has not acted on yet. The host keeps the last context of a page and hands it to every mount of
 * it, so each `panelContext.id` is acted on once per console tab: a remount must not reopen a dialog or reselect.
 */
export function takePanelRequest(panelContext: PageRenderProps['panelContext']): { id?: string; action?: string } {
  if (!panelContext || handledRequests.has(panelContext.id)) return {}
  handledRequests.add(panelContext.id)
  return panelRequest(panelContext)
}

/** Open a page of this worker beside the caller; false when the console has no panes. */
export function openPane(host: Host, pageId: string, context?: Record<string, string>): boolean {
  if (!host.panels?.open) return false
  host.panels.open({ pageId, context })
  return true
}

const submitOnEnter = (submit: () => void) => (event: ReactKeyboardEvent) => {
  if (event.key === 'Enter' && !event.shiftKey && event.target instanceof HTMLInputElement) {
    event.preventDefault()
    submit()
  }
}

/** The server's refusal of a name (taken, empty) belongs under the name field; anything else under the form. */
const isNameError = (message: string) => /\bname|exist|already|unique/i.test(message)

/* ── new release ──────────────────────────────────────────────────────── */

export function NewReleaseDialog({
  host,
  open,
  onOpenChange,
  onCreated,
}: {
  host: Host
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated: (release: ReleaseView) => void
}) {
  const [name, setName] = useState('')
  const [target, setTarget] = useState('')
  const [description, setDescription] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    setName('')
    setTarget('')
    setDescription('')
    setBusy(false)
    setError(null)
  }, [open])

  const targetProblem = target.trim() !== '' && !isIsoDay(target.trim()) ? 'Use the form YYYY-MM-DD' : null
  const canSubmit = name.trim().length > 0 && !targetProblem && !busy

  const submit = async () => {
    if (!canSubmit) return
    setBusy(true)
    setError(null)
    try {
      const release = await api.createRelease(host.iii, {
        name: name.trim(),
        description: description.trim(),
        ...(target.trim() ? { target_at: target.trim() } : {}),
      })
      announcePlanChange()
      onCreated(release)
    } catch (caught) {
      setError(errorMessage(caught))
    } finally {
      setBusy(false)
    }
  }

  const nameError = error && isNameError(error) ? error : null

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="kanban-dialog">
        <DialogTitle>New release</DialogTitle>
        <DialogDescription>Group tickets into a delivery with a target date.</DialogDescription>
        <div className="kanban-dialog__form" onKeyDown={submitOnEnter(() => void submit())}>
          <div className="kanban-plan-pair">
            <label className="kanban-field">
              <span className="kanban-plan-labelrow">
                <span className="kanban-field__label">Name</span>
                <span className="kanban-plan-opt-note">Required</span>
              </span>
              <Input
                aria-describedby={nameError ? 'kanban-new-release-name-error' : undefined}
                aria-invalid={nameError ? true : undefined}
                autoFocus
                onChange={(next) => {
                  setName(next)
                  setError(null)
                }}
                placeholder="Phase 3"
                value={name}
              />
              {nameError ? (
                <p className="kanban-plan-error" id="kanban-new-release-name-error" role="alert">
                  <TriangleAlert size={16} />
                  {nameError}
                </p>
              ) : null}
            </label>
            <label className="kanban-field">
              <span className="kanban-plan-labelrow">
                <span className="kanban-field__label">Target date</span>
                <span className="kanban-plan-opt-note">Optional</span>
              </span>
              <Input
                aria-invalid={targetProblem ? true : undefined}
                className="kanban-plan-mono"
                onChange={setTarget}
                placeholder="YYYY-MM-DD"
                value={target}
              />
              {targetProblem ? <p className="kanban-field__error">{targetProblem}</p> : null}
            </label>
          </div>
          <label className="kanban-field">
            <span className="kanban-plan-labelrow">
              <span className="kanban-field__label">Description</span>
              <span className="kanban-plan-opt-note">Optional</span>
            </span>
            <textarea
              className="kanban-field__textarea"
              onChange={(event) => setDescription(event.target.value)}
              placeholder="What this release delivers"
              rows={3}
              value={description}
            />
          </label>
          {error && !nameError ? <p className="kanban-field__error">{error}</p> : null}
        </div>
        <div className="kanban-dialog__foot">
          <Button onClick={() => onOpenChange(false)} variant="ghost">
            Cancel
          </Button>
          <Button disabled={!canSubmit} onClick={() => void submit()} variant="primary">
            {busy ? 'Creating…' : 'Create release'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

/* ── new project ──────────────────────────────────────────────────────── */

export function NewProjectDialog({
  host,
  open,
  onOpenChange,
  agents,
  onCreated,
}: {
  host: Host
  open: boolean
  onOpenChange: (open: boolean) => void
  agents: AgentProfile[]
  onCreated: (project: ProjectView) => void
}) {
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [lead, setLead] = useState<string | undefined>(undefined)
  const [color, setColor] = useState<GlyphTone>('blue')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const colorLabel = useId()

  useEffect(() => {
    if (!open) return
    setName('')
    setDescription('')
    setLead(undefined)
    setColor('blue')
    setBusy(false)
    setError(null)
  }, [open])

  const canSubmit = name.trim().length > 0 && !busy

  const submit = async () => {
    if (!canSubmit) return
    setBusy(true)
    setError(null)
    try {
      const project = await api.createProject(host.iii, {
        name: name.trim(),
        description: description.trim(),
        color,
        ...(lead ? { lead } : {}),
      })
      announcePlanChange()
      onCreated(project)
    } catch (caught) {
      setError(errorMessage(caught))
    } finally {
      setBusy(false)
    }
  }

  const nameError = error && isNameError(error) ? error : null

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="kanban-dialog">
        <DialogTitle>New project</DialogTitle>
        <DialogDescription>A ticket belongs to at most one project.</DialogDescription>
        <div className="kanban-dialog__form" onKeyDown={submitOnEnter(() => void submit())}>
          <label className="kanban-field">
            <span className="kanban-plan-labelrow">
              <span className="kanban-field__label">Name</span>
              <span className="kanban-plan-opt-note">Required</span>
            </span>
            <Input
              aria-describedby="kanban-new-project-name-hint"
              aria-invalid={nameError ? true : undefined}
              autoFocus
              onChange={(next) => {
                setName(next)
                setError(null)
              }}
              placeholder="iii cloud"
              value={name}
            />
            {nameError ? (
              <p className="kanban-plan-error" role="alert">
                <TriangleAlert size={16} />
                {nameError}
              </p>
            ) : (
              <span className="kanban-plan-hint" id="kanban-new-project-name-hint">
                Unique on this board. Case is ignored.
              </span>
            )}
          </label>
          <label className="kanban-field">
            <span className="kanban-plan-labelrow">
              <span className="kanban-field__label">Description</span>
              <span className="kanban-plan-opt-note">Optional</span>
            </span>
            <textarea
              className="kanban-field__textarea"
              onChange={(event) => setDescription(event.target.value)}
              placeholder="What is this project for?"
              rows={4}
              value={description}
            />
          </label>
          <div className="kanban-plan-pair">
            <div className="kanban-field">
              <span className="kanban-plan-labelrow">
                <span className="kanban-field__label">Lead</span>
                <span className="kanban-plan-opt-note">Optional</span>
              </span>
              <Selector
                allowEmpty
                aria-label="Lead"
                emptyLabel="None"
                onChange={setLead}
                onClear={() => setLead(undefined)}
                options={agentOptions(agents)}
                placeholder="None"
                value={lead}
              />
            </div>
            <div className="kanban-field">
              <span className="kanban-field__label" id={colorLabel}>
                Color
              </span>
              <div aria-labelledby={colorLabel} className="kanban-plan-tones" role="radiogroup">
                {TONES.map((tone) => (
                  <button
                    aria-checked={color === tone}
                    className="kanban-plan-tone"
                    key={tone}
                    onClick={() => setColor(tone)}
                    role="radio"
                    type="button"
                  >
                    <ProjectGlyph color={tone} />
                    <span className="kanban-plan-tone__name">{titleCase(tone)}</span>
                    {color === tone ? <Check size={16} /> : <span className="kanban-plan-tone__gap" />}
                  </button>
                ))}
              </div>
            </div>
          </div>
          {error && !nameError ? <p className="kanban-field__error">{error}</p> : null}
        </div>
        <div className="kanban-dialog__foot">
          <Button onClick={() => onOpenChange(false)} variant="ghost">
            Cancel
          </Button>
          <Button disabled={!canSubmit} onClick={() => void submit()} variant="primary">
            {busy ? 'Creating…' : 'Create project'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

/* ── mark as released ─────────────────────────────────────────────────── */

export function ShipDialog({
  host,
  open,
  onOpenChange,
  release,
  releases,
  onShipped,
}: {
  host: Host
  open: boolean
  onOpenChange: (open: boolean) => void
  release: ReleaseView
  releases: ReleaseView[]
  onShipped: (result: { release: ReleaseView; moved: TicketSummary[]; to: ReleaseView | null }) => void
}) {
  const targets = useMemo(() => shipTargets(releases, release.id), [releases, release.id])
  const next = useMemo(() => nextRelease(releases, release.id), [releases, release.id])
  // The board reloads under an open dialog and rebuilds `next` each time: the default is read when the dialog opens,
  // never again, or it would undo a choice made since.
  const nextRef = useRef(next)
  nextRef.current = next
  const [dest, setDest] = useState<'move' | 'remove'>('remove')
  const [targetId, setTargetId] = useState<string | undefined>(undefined)
  const [busy, setBusy] = useState(false)
  const busyRef = useRef(false)
  const [error, setError] = useState<string | null>(null)
  const group = useId()
  const unfinished = notDone(release)

  useEffect(() => {
    if (!open) return
    setDest(nextRef.current ? 'move' : 'remove')
    setTargetId(nextRef.current?.id)
    setError(null)
  }, [open])

  const target = targets.find((candidate) => candidate.id === targetId) ?? null
  const to = unfinished > 0 && dest === 'move' ? target : null
  // "Move them" with nowhere to move to (the chosen release was released meanwhile) must not turn into "remove".
  const blocked = unfinished > 0 && dest === 'move' && !target

  const submit = async () => {
    if (busyRef.current || blocked) return
    busyRef.current = true
    setBusy(true)
    setError(null)
    try {
      const result = await api.shipRelease(host.iii, {
        id: release.id,
        ...(unfinished > 0 ? { move_open_to: to ? to.id : null } : {}),
      })
      announcePlanChange()
      onShipped({ ...result, to })
    } catch (caught) {
      setError(errorMessage(caught))
    } finally {
      busyRef.current = false
      setBusy(false)
    }
  }

  const holds = target ? target.total + unfinished : 0

  return (
    <Dialog onOpenChange={(next_) => !busy && onOpenChange(next_)} open={open}>
      <DialogContent className="kanban-dialog kanban-plan-dialog">
        <DialogTitle>Mark {release.name} as released?</DialogTitle>
        <DialogDescription>
          {unfinished > 0
            ? 'This closes the release. Choose where its unfinished tickets go.'
            : 'This closes the release.'}
        </DialogDescription>
        <div className="kanban-dialog__form">
          <ul aria-label="What happens to the tickets" className="kanban-plan-facts">
            {release.total === 0 ? (
              <li>
                <Info size={16} />
                <span>{release.name} has no tickets.</span>
              </li>
            ) : (
              <>
                <li>
                  <span className="kanban-plan-dot" data-tone="ok" />
                  <span>
                    <span className="kanban-plan-mono">{release.done}</span> done{' '}
                    {release.done === 1 ? 'ticket stays' : 'tickets stay'} in {release.name}
                  </span>
                </li>
                {unfinished > 0 ? (
                  <li>
                    <span className="kanban-plan-dot" data-tone="ghost" />
                    <span>
                      <span className="kanban-plan-mono">{unfinished}</span> {unfinished === 1 ? 'is' : 'are'} not done
                      and must go somewhere
                    </span>
                  </li>
                ) : null}
              </>
            )}
          </ul>

          {unfinished > 0 ? (
            <fieldset className="kanban-plan-fieldset">
              <legend className="kanban-field__label">Where do they go?</legend>
              <label className="kanban-plan-opt" data-on={dest === 'move'}>
                <input
                  checked={dest === 'move'}
                  disabled={targets.length === 0}
                  name={group}
                  onChange={() => setDest('move')}
                  type="radio"
                />
                <span className="kanban-plan-opt__body">
                  <span className="kanban-plan-opt__line">
                    Move them to
                    {targets.length > 0 ? (
                      <Selector
                        aria-label="Release to move the tickets to"
                        className="kanban-plan-opt__select"
                        onChange={(value) => {
                          setTargetId(value)
                          setDest('move')
                        }}
                        options={targets.map((candidate) => ({
                          value: candidate.id,
                          label: candidate.name,
                          description: candidate.target_at ? `Target ${formatTarget(candidate.target_at)}` : undefined,
                        }))}
                        searchPlaceholder="Search releases"
                        value={targetId}
                      />
                    ) : null}
                  </span>
                  <span className="kanban-plan-hint">
                    {target
                      ? `${target.id === next?.id && isNextByDate(target, release) ? `${target.name} is the next release by target date. ` : ''}It will hold ${plural(holds, 'ticket')}.`
                      : targets.length > 0
                        ? 'Pick the release they move to.'
                        : 'There is no other release that is not released yet.'}
                  </span>
                </span>
              </label>
              <label className="kanban-plan-opt" data-on={dest === 'remove'}>
                <input checked={dest === 'remove'} name={group} onChange={() => setDest('remove')} type="radio" />
                <span className="kanban-plan-opt__body">
                  <span className="kanban-plan-opt__line">Remove them from any release</span>
                  <span className="kanban-plan-hint">They stay on the board with no release.</span>
                </span>
              </label>
            </fieldset>
          ) : null}

          <ul className="kanban-plan-notes">
            {unfinished > 0 ? (
              <li>
                <Bell size={16} />
                <span>Agents watching {release.name} are notified for each moved ticket</span>
              </li>
            ) : null}
            <li>
              <RotateCcw size={16} />
              <span>You can reopen the release later{unfinished > 0 ? "; moved tickets don't come back" : ''}</span>
            </li>
          </ul>
          {error ? <p className="kanban-field__error">{error}</p> : null}
        </div>
        <div className="kanban-dialog__foot">
          {/* A ConfirmDialog cannot hold the radios; this one gives Cancel the focus the way it does. */}
          <Button autoFocus disabled={busy} onClick={() => onOpenChange(false)} variant="ghost">
            Cancel
          </Button>
          <Button disabled={busy || blocked} onClick={() => void submit()} variant="primary">
            {busy ? 'Marking…' : 'Mark as released'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

/* ── create from a label ──────────────────────────────────────────────── */

type Into = 'release' | 'project'
type Preview = ConvertResult | { error: string }
const failed = (preview: Preview | undefined): preview is { error: string } =>
  preview !== undefined && 'error' in preview

export type ConvertOutcome = {
  into: Into
  /** What the runs touched, the label you picked first. */
  entities: (ReleaseView | ProjectView)[]
  text: string
}

/**
 * "Create from a label…". Pick a label, then create a new release/project from its tickets or add them to an
 * existing one. A label that belongs to a family (`phase-0` … `phase-6`) also offers one entity per label, each
 * with an editable name. Every number comes from a dry run of the very call the button makes.
 */
export function ConvertDialog({
  host,
  open,
  onOpenChange,
  into,
  data,
  onDone,
}: {
  host: Host
  open: boolean
  onOpenChange: (open: boolean) => void
  into: Into
  data: BoardData
  onDone: (outcome: ConvertOutcome) => void
}) {
  const noun = into
  const uid = useId()
  const tickets = useMemo(() => data.columns.flatMap((column) => column.tickets), [data])
  const labels = useMemo(() => labelCounts(tickets), [tickets])
  const labelNames = useMemo(() => labels.map((entry) => entry.label), [labels])
  const entities = useMemo<(ReleaseView | ProjectView)[]>(
    () => (into === 'release' ? data.releases : data.projects.filter((project) => project.status !== 'archived')),
    [into, data],
  )

  const [label, setLabel] = useState<string | undefined>(undefined)
  const [mode, setMode] = useState<'new' | 'existing'>('new')
  const [names, setNames] = useState<Record<string, string>>({})
  /** Rows the user switched on; the label picked is always in, its siblings start out. */
  const [picked, setPicked] = useState<Record<string, boolean>>({})
  /** The mode was set by picking a label whose name is taken, not by the user: the next label may undo it. */
  const autoExisting = useRef(false)
  const [target, setTarget] = useState('')
  const [existingId, setExistingId] = useState<string | undefined>(undefined)
  const [replace, setReplace] = useState(false)
  const [markReleased, setMarkReleased] = useState(false)
  const [removeLabel, setRemoveLabel] = useState(false)
  const [previews, setPreviews] = useState<Record<string, Preview>>({})
  const [previewing, setPreviewing] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    setLabel(undefined)
    setMode('new')
    setNames({})
    setPicked({})
    autoExisting.current = false
    setTarget('')
    setExistingId(undefined)
    setReplace(false)
    setMarkReleased(false)
    setRemoveLabel(false)
    setPreviews({})
    setBusy(false)
    setError(null)
  }, [open])

  const switchMode = (next: 'new' | 'existing') => {
    setMode(next)
    setPreviews({})
    setReplace(false)
  }

  const chooseLabel = (next: string) => {
    setLabel(next)
    setNames({})
    setPicked({})
    setMarkReleased(false)
    setPreviews({})
    setError(null)
    // A name that is taken means the tickets belong in that one: go straight to "add to an existing".
    const taken = resolveEntity(entities, suggestName(next))
    if (taken) {
      switchMode('existing')
      setExistingId(taken.id)
      autoExisting.current = true
    } else if (autoExisting.current) {
      switchMode('new')
      setExistingId(undefined)
      autoExisting.current = false
    }
  }

  const key = asciiLower
  const nameOf = (entry: string) => names[key(entry)] ?? suggestName(entry)
  const isChosen = (entry: string) => label !== undefined && key(entry) === key(label)
  const isOn = (entry: string) => isChosen(entry) || Boolean(picked[key(entry)])

  const family = useMemo(
    () => (label && mode === 'new' ? labelFamily(label, labelNames) : []),
    [label, mode, labelNames],
  )
  const tableRows = label ? (family.length > 1 ? family : [label]) : []
  const selected = tableRows.filter(isOn)
  const batch = tableRows.length > 1
  // Only in "add to an existing" mode: a choice made before switching to "new" is not this run's target.
  const existing = mode === 'existing' ? entities.find((entity) => entity.id === existingId) : undefined

  const stats = useMemo(
    () =>
      Object.fromEntries(
        tableRows.map((entry) => {
          const own = ticketsWithLabel(tickets, entry)
          return [
            key(entry),
            { total: own.length, done: own.filter((ticket) => ticket.status === data.done_status).length },
          ]
        }),
      ),
    [tickets, label, family, data.done_status],
  )

  // The dry runs: one per row, once the names stop changing. The label picked is part of the key: every member of
  // a family has the same rows, and switching label clears the previews, so the effect must run again.
  const planKey = JSON.stringify([
    label ?? null,
    tableRows.map((entry) =>
      mode === 'existing' ? { entry, id: existingId ?? null } : { entry, name: nameOf(entry).trim() },
    ),
  ])
  const settled = useDebounce(planKey, 300)
  useEffect(() => {
    if (!open) return
    const [, plan] = JSON.parse(settled) as [string | null, { entry: string; id?: string | null; name?: string }[]]
    const ready = plan.filter((row) => (row.id !== undefined ? Boolean(row.id) : Boolean(row.name)))
    if (ready.length === 0) {
      setPreviews({})
      setPreviewing(false)
      return
    }
    let cancelled = false
    setPreviewing(true)
    void Promise.all(
      ready.map(async (row) => {
        try {
          const result = await api.convertLabel(host.iii, {
            label: row.entry,
            into,
            ...(row.id ? { id: row.id } : { name: row.name }),
            dry_run: true,
          })
          return [key(row.entry), result] as const
        } catch (caught) {
          return [key(row.entry), { error: errorMessage(caught) }] as const
        }
      }),
    ).then((pairs) => {
      if (cancelled) return
      setPreviews(Object.fromEntries(pairs))
      setPreviewing(false)
    })
    return () => {
      cancelled = true
    }
    // `data` too: a run that ended halfway reloads the board, and the numbers on screen are of before it.
  }, [settled, open, host, into, data])

  const pending = planKey !== settled || previewing
  const previewOf = (entry: string): ConvertResult | undefined => {
    const preview = previews[key(entry)]
    return preview && !failed(preview) ? preview : undefined
  }
  const chosenPreview = label ? previews[key(label)] : undefined
  const sumAdd = selected.reduce((sum, entry) => {
    const preview = previewOf(entry)
    return sum + (preview ? willAdd(preview.buckets, replace) : 0)
  }, 0)
  const alreadyThere = selected.filter((entry) => previewOf(entry)?.entity).length
  const inOther = selected.reduce((sum, entry) => sum + (previewOf(entry)?.buckets.in_other.length ?? 0), 0)
  const labelTicketCount = new Set(
    selected.flatMap((entry) => ticketsWithLabel(tickets, entry).map((ticket) => ticket.id)),
  ).size
  const releasable = selected.filter((entry) => {
    const own = stats[key(entry)]
    return own && own.total > 0 && own.total === own.done
  })

  // Off the moment it cannot apply (no selected label is all done), whatever was ticked before.
  const markDisabled = mode === 'new' && releasable.length === 0
  const markNow = markReleased && !markDisabled

  const targetProblem =
    into === 'release' && mode === 'new' && target.trim() !== '' && !isIsoDay(target.trim())
      ? 'Use the form YYYY-MM-DD'
      : null
  const missingName = mode === 'new' && selected.some((entry) => nameOf(entry).trim() === '')
  const reason = !label
    ? 'Pick a label'
    : mode === 'existing' && !existingId
      ? `Pick a ${noun}`
      : missingName
        ? `Name the ${noun}`
        : targetProblem
          ? targetProblem
          : !pending && selected.some((entry) => failed(previews[key(entry)]))
            ? 'The preview failed'
            : !pending && sumAdd === 0
              ? 'Nothing to add'
              : null
  const canRun = reason === null && !pending && !busy

  const confirmLabel = !label
    ? `Create ${noun}`
    : mode === 'existing'
      ? `Add ${plural(sumAdd, 'ticket')} to ${existing?.name ?? `the ${noun}`}`
      : selected.length === 1
        ? alreadyThere
          ? `Add ${plural(sumAdd, 'ticket')} to ${(previewOf(selected[0])?.entity as { name: string }).name}`
          : `Create ${noun} · ${plural(sumAdd, 'ticket')}`
        : `${alreadyThere === 0 ? 'Create' : 'Apply to'} ${plural(selected.length, noun)} · ${plural(sumAdd, 'ticket')}`

  const run = async () => {
    if (!canRun || !label) return
    setBusy(true)
    setError(null)
    // The label you picked runs first: it is the one that opens afterwards.
    const order = [label, ...selected.filter((entry) => !isChosen(entry))]
    const results: ConvertResult[] = []
    try {
      for (const entry of order) {
        results.push(
          await api.convertLabel(host.iii, {
            label: entry,
            into,
            ...(mode === 'existing' ? { id: existingId } : { name: nameOf(entry).trim() }),
            ...(into === 'release' && mode === 'new' && isChosen(entry) && target.trim()
              ? { target_at: target.trim() }
              : {}),
            replace,
            remove_label: removeLabel,
            ...(into === 'release' ? { mark_released_if_done: markNow } : {}),
          }),
        )
      }
    } catch (caught) {
      setError(
        `${results.length > 0 ? `Done for ${results.length} of ${order.length}, then it stopped: ` : ''}${errorMessage(caught)}`,
      )
      if (results.length > 0) {
        announcePlanChange()
        onDone(outcome(results))
      }
      setBusy(false)
      return
    }
    announcePlanChange()
    onDone(outcome(results))
    setBusy(false)
    onOpenChange(false)
  }

  const outcome = (results: ConvertResult[]): ConvertOutcome => {
    const touched = results
      .map((result) => result.entity)
      .filter((entity): entity is ReleaseView | ProjectView => entity !== null)
    const added = results.reduce((sum, result) => sum + result.tickets.length, 0)
    const created = results.filter((result) => result.created).length
    const text =
      mode === 'existing' || (results.length === 1 && created === 0)
        ? `Added ${plural(added, 'ticket')} to ${touched[0]?.name ?? `the ${noun}`}`
        : `Created ${plural(created, noun)} · ${plural(added, 'ticket')}${created < results.length ? ` · ${results.length - created} already existed` : ''}`
    return { into, entities: touched, text }
  }

  const columns = data.columns
  const chosenLabel = label ? labels.find((entry) => key(entry.label) === key(label)) : undefined
  const labelSummary = label ? columnSummary(ticketsWithLabel(tickets, label), columns) : ''
  const buckets = chosenPreview && !failed(chosenPreview) ? chosenPreview.buckets : null
  const heading = existing && mode === 'existing' ? ` in ${existing.name}` : ''
  const stem = label ? `${label.replace(/\d+$/, '')}*` : ''
  const everyOn = tableRows.every(isOn)
  const someOn = tableRows.some((entry) => !isChosen(entry) && isOn(entry))

  return (
    <Dialog onOpenChange={(next) => !busy && onOpenChange(next)} open={open}>
      <DialogContent className="kanban-dialog kanban-plan-dialog kanban-plan-dialog--wide">
        <DialogTitle>Create from a label</DialogTitle>
        <DialogDescription>Puts the tickets that carry a label into a {noun}. Nothing is deleted.</DialogDescription>
        <div className="kanban-dialog__form">
          <div className="kanban-plan-pair kanban-plan-pair--wide">
            <div className="kanban-field">
              <span className="kanban-field__label">Label</span>
              <WithCount count={chosenLabel ? plural(chosenLabel.count, 'ticket') : undefined}>
                <Selector
                  aria-label="Label"
                  emptyMessage="No ticket carries that label"
                  onChange={chooseLabel}
                  options={labels.map((entry) => ({
                    value: entry.label,
                    label: entry.label,
                    description: plural(entry.count, 'ticket'),
                  }))}
                  placeholder={labels.length === 0 ? 'No ticket has a label yet' : 'Pick a label'}
                  searchPlaceholder="Search labels"
                  triggerIcon={<Tag size={16} />}
                  value={label}
                />
              </WithCount>
            </div>
            <div className="kanban-field">
              <span className="kanban-field__label" id={`${uid}-into`}>
                Put them in
              </span>
              <SegmentedControl
                aria-label="Put them in"
                onChange={(next) => {
                  autoExisting.current = false
                  switchMode(next)
                }}
                options={[
                  { value: 'new', label: `Create a new ${noun}`, icon: false },
                  { value: 'existing', label: `Add to an existing ${noun}`, icon: false },
                ]}
                variant="radio"
                value={mode}
              />
            </div>
            {mode === 'new' ? (
              <>
                <label className="kanban-field">
                  <span className="kanban-field__label">{titleCase(noun)} name</span>
                  <Input
                    disabled={!label}
                    onChange={(next) => label && setNames((current) => ({ ...current, [key(label)]: next }))}
                    placeholder={label ? suggestName(label) : ''}
                    value={label ? nameOf(label) : ''}
                  />
                  {chosenPreview && !failed(chosenPreview) && chosenPreview.entity ? (
                    <span className="kanban-plan-hint" role="status">
                      A {noun} named {chosenPreview.entity.name} already exists. The tickets are added to it.
                    </span>
                  ) : null}
                </label>
                {into === 'release' ? (
                  <label className="kanban-field">
                    <span className="kanban-plan-labelrow">
                      <span className="kanban-field__label">Target date</span>
                      <span className="kanban-plan-opt-note">Optional</span>
                    </span>
                    <Input
                      aria-invalid={targetProblem ? true : undefined}
                      className="kanban-plan-mono"
                      onChange={setTarget}
                      placeholder="YYYY-MM-DD"
                      value={target}
                    />
                    {targetProblem ? (
                      <span className="kanban-field__error">{targetProblem}</span>
                    ) : batch && label ? (
                      <span className="kanban-plan-hint">Only for {nameOf(label)}. Set the others on their pages.</span>
                    ) : null}
                  </label>
                ) : null}
              </>
            ) : (
              <>
                <div className="kanban-field">
                  <span className="kanban-field__label">{titleCase(noun)}</span>
                  <WithCount count={existing ? plural(existing.total, 'ticket') : undefined}>
                    <Selector
                      aria-label={titleCase(noun)}
                      onChange={setExistingId}
                      options={entities.map((entity) => ({
                        value: entity.id,
                        label: entity.name,
                        description: plural(entity.total, 'ticket'),
                      }))}
                      placeholder={entities.length === 0 ? `No ${noun} yet` : `Pick a ${noun}`}
                      searchPlaceholder={`Search ${noun}s`}
                      triggerIcon={into === 'release' ? <Flag size={16} /> : <Folder size={16} />}
                      value={existingId}
                    />
                  </WithCount>
                </div>
                {existing ? (
                  <div className="kanban-plan-status">
                    <span
                      className="kanban-plan-dot"
                      data-tone={
                        existing.status === 'released'
                          ? 'ok'
                          : existing.status === 'planned' || existing.status === 'archived'
                            ? 'ghost'
                            : 'ink'
                      }
                    />
                    {titleCase(existing.status)}
                    {'target_at' in existing && existing.target_at
                      ? ` · target ${formatTarget(existing.target_at)}`
                      : ''}
                  </div>
                ) : null}
              </>
            )}
          </div>

          {label ? (
            <section aria-label={`Preview for ${label}`} className="kanban-plan-preview">
              <div className="kanban-plan-preview__head">
                <h3>
                  Preview for <span className="kanban-plan-mono">{label}</span>
                  {heading}
                </h3>
                <span className="kanban-plan-hint">{labelSummary}</span>
              </div>
              {chosenPreview && failed(chosenPreview) ? (
                <p className="kanban-field__error" role="alert">
                  {chosenPreview.error}
                </p>
              ) : buckets ? (
                <div className="kanban-plan-buckets">
                  <Bucket
                    n={buckets.added.length}
                    note={`Go into ${existing?.name ?? chosenName(chosenPreview, nameOf(label), noun)}`}
                    title="will be added"
                  />
                  <Bucket
                    muted={buckets.already_here.length === 0}
                    n={buckets.already_here.length}
                    note="Left as they are"
                    title="already here"
                  />
                  <Bucket
                    muted={buckets.in_other.length === 0}
                    n={buckets.in_other.length}
                    note={
                      <Checkbox
                        checked={replace}
                        disabled={inOther === 0}
                        label="Move them here"
                        onChange={(event) => setReplace(event.target.checked)}
                      />
                    }
                    title={`in another ${noun}`}
                  />
                  <Bucket
                    muted={buckets.deleted.length === 0}
                    n={buckets.deleted.length}
                    note="Skipped"
                    title="deleted"
                  />
                </div>
              ) : (
                <div className="kanban-plan-buckets">
                  {[0, 1, 2, 3].map((index) => (
                    <Skeleton className="kanban-plan-skeleton" key={index} />
                  ))}
                </div>
              )}
            </section>
          ) : null}

          {batch ? (
            <Panel aria-labelledby={`${uid}-batch`} className="kanban-plan-batch" role="region">
              <PanelHeader>
                <h3 id={`${uid}-batch`}>One {noun} for each matching label</h3>
                <span className="kanban-plan-mono kanban-plan-hint">{stem}</span>
                <span className="kanban-plan-hint kanban-plan-batch__count">
                  {selected.length} of {tableRows.length} selected
                </span>
              </PanelHeader>
              <TableViewport>
                <TableFrame>
                  <Table density="compact" inset>
                    <TableHeader>
                      <TableRow>
                        <TableHead>
                          <Checkbox
                            aria-label={`Select all ${tableRows.length} labels`}
                            checked={everyOn}
                            indeterminate={!everyOn && someOn}
                            onChange={() =>
                              setPicked(everyOn ? {} : Object.fromEntries(tableRows.map((entry) => [key(entry), true])))
                            }
                          />
                        </TableHead>
                        <TableHead>Label</TableHead>
                        <TableHead>{titleCase(noun)} name</TableHead>
                        <TableHead className="kanban-plan-num">Tickets</TableHead>
                        <TableHead className="kanban-plan-num">Done</TableHead>
                        <TableHead className="kanban-plan-num">Will add</TableHead>
                        <TableHead className="kanban-plan-num">Already here</TableHead>
                        <TableHead className="kanban-plan-num">In another</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {tableRows.map((entry) => {
                        const preview = previewOf(entry)
                        const on = isOn(entry)
                        const own = stats[key(entry)]
                        return (
                          <TableRow
                            aria-current={isChosen(entry) ? 'true' : undefined}
                            data-off={on ? undefined : 'true'}
                            key={key(entry)}
                          >
                            <TableCell>
                              <Checkbox
                                aria-label={`Create a ${noun} for ${entry}`}
                                checked={on}
                                disabled={isChosen(entry)}
                                onChange={(event) =>
                                  setPicked((current) => ({ ...current, [key(entry)]: event.target.checked }))
                                }
                              />
                            </TableCell>
                            <TableCell className="kanban-plan-mono">{entry}</TableCell>
                            <TableCell>
                              <Input
                                aria-label={`${titleCase(noun)} name for ${entry}`}
                                onChange={(next) => setNames((current) => ({ ...current, [key(entry)]: next }))}
                                value={nameOf(entry)}
                              />
                            </TableCell>
                            <TableCell className="kanban-plan-num kanban-plan-mono">{own?.total ?? 0}</TableCell>
                            <TableCell className="kanban-plan-num kanban-plan-mono">{own?.done ?? 0}</TableCell>
                            <TableCell className="kanban-plan-num kanban-plan-mono">
                              {preview ? willAdd(preview.buckets, replace) : '…'}
                            </TableCell>
                            <TableCell className="kanban-plan-num kanban-plan-mono">
                              {preview ? preview.buckets.already_here.length : '…'}
                            </TableCell>
                            <TableCell className="kanban-plan-num kanban-plan-mono">
                              {preview ? preview.buckets.in_other.length : '…'}
                            </TableCell>
                          </TableRow>
                        )
                      })}
                    </TableBody>
                    <TableFooter>
                      <TableRow>
                        <TableCell />
                        <TableCell>Total</TableCell>
                        <TableCell>{plural(selected.length, noun)}</TableCell>
                        <TableCell className="kanban-plan-num kanban-plan-mono">{labelTicketCount}</TableCell>
                        <TableCell className="kanban-plan-num kanban-plan-mono">
                          {selected.reduce((sum, entry) => sum + (stats[key(entry)]?.done ?? 0), 0)}
                        </TableCell>
                        <TableCell className="kanban-plan-num kanban-plan-mono">{sumAdd}</TableCell>
                        <TableCell className="kanban-plan-num kanban-plan-mono">
                          {selected.reduce(
                            (sum, entry) => sum + (previewOf(entry)?.buckets.already_here.length ?? 0),
                            0,
                          )}
                        </TableCell>
                        <TableCell className="kanban-plan-num kanban-plan-mono">{inOther}</TableCell>
                      </TableRow>
                    </TableFooter>
                  </Table>
                </TableFrame>
              </TableViewport>
            </Panel>
          ) : null}

          {label ? (
            <div className="kanban-plan-checks">
              {into === 'release' ? (
                <Checkbox
                  checked={markNow}
                  disabled={markDisabled}
                  label={
                    <span className="kanban-plan-check">
                      <span>Mark releases whose tickets are all done as Released</span>
                      <span className="kanban-plan-hint">
                        {mode === 'existing'
                          ? `When every ticket of ${existing?.name ?? 'the release'} is done`
                          : releasable.length > 0
                            ? `${plural(releasable.length, 'release')}: ${nameList(releasable.map(nameOf))}`
                            : 'No selected label has all its tickets done'}
                      </span>
                    </span>
                  }
                  onChange={(event) => setMarkReleased(event.target.checked)}
                />
              ) : null}
              <Checkbox
                checked={removeLabel}
                label={
                  <span className="kanban-plan-check">
                    <span>Remove the label from these tickets</span>
                    <span className="kanban-plan-hint">
                      {removeLabel
                        ? `Off the tickets that end up in the ${noun}`
                        : `Off: ${selected.length > 1 ? `${selected[0]} … ${selected[selected.length - 1]} stay` : `${selected[0] ?? label} stays`} on all ${labelTicketCount} ${labelTicketCount === 1 ? 'ticket' : 'tickets'}`}
                    </span>
                  </span>
                }
                onChange={(event) => setRemoveLabel(event.target.checked)}
              />
            </div>
          ) : null}

          {error ? (
            <p className="kanban-field__error" role="alert">
              {error}
            </p>
          ) : null}
        </div>
        <div className="kanban-dialog__foot">
          <Button disabled={busy} onClick={() => onOpenChange(false)} variant="ghost">
            Cancel
          </Button>
          {reason && label ? (
            <span className="kanban-plan-reason" id={`${uid}-why`}>
              <Info size={16} />
              {reason}
            </span>
          ) : null}
          <Button
            aria-describedby={reason ? `${uid}-why` : undefined}
            disabled={!canRun}
            onClick={() => void run()}
            variant="primary"
          >
            {busy ? 'Working…' : confirmLabel}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function chosenName(preview: Preview | undefined, fallback: string, noun: string): string {
  if (preview && !failed(preview) && preview.entity) return preview.entity.name
  return fallback.trim() || `the ${noun}`
}

/** The host's Selector trigger shows only the label: the count sits over its right edge, clear of the chevron. */
function WithCount({ count, children }: { count?: string; children: ReactNode }) {
  if (!count) return <>{children}</>
  return (
    <div className="kanban-plan-withcount">
      {children}
      <span aria-hidden="true" className="kanban-plan-withcount__n">
        {count}
      </span>
    </div>
  )
}

function Bucket({ n, title, note, muted }: { n: number; title: string; note: ReactNode; muted?: boolean }) {
  return (
    <div className="kanban-plan-bucket">
      <div className="kanban-plan-bucket__top">
        <span className="kanban-plan-bucket__n" data-muted={muted ? 'true' : undefined}>
          {n}
        </span>
        <span>{title}</span>
      </div>
      <div className="kanban-plan-bucket__note">{note}</div>
    </div>
  )
}

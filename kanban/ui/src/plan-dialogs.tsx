import {
  type Host,
  type PageRenderProps,
  ShadcnBadge,
  ShadcnButton,
  ShadcnCheckbox,
  ShadcnDialog,
  ShadcnDialogContent,
  ShadcnDialogDescription,
  ShadcnDialogFooter,
  ShadcnDialogHeader,
  ShadcnDialogTitle,
  ShadcnInput,
  ShadcnProgress,
  ShadcnSkeleton,
  ShadcnTable,
  ShadcnTableBody,
  ShadcnTableCell,
  ShadcnTableHead,
  ShadcnTableHeader,
  ShadcnTableRow,
  ShadcnTabs,
  ShadcnTabsList,
  ShadcnTabsTrigger,
  ShadcnTextarea,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useDebounce } from '@iii-dev/console-ui/hooks'
import { Bell, CircleAlert, Info, Milestone, RotateCcw, Tag } from 'lucide-react'
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
  allClosed,
  asciiLower,
  columnSummary,
  formatTarget,
  isIsoDay,
  isNextByDate,
  labelCounts,
  labelFamily,
  liveCount,
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
  subscribeChanges,
  type TicketSummary,
} from './shared'
import { Crumbs, FolderGlyph, LeadPicker, PickerButton, PROJECT_TONES } from './plan/parts'
import { Field } from './ui/feedback'
import { ColumnStatusIcon } from './ui/glyphs'
import { type PickerOption, SearchPicker } from './ui/search-picker'
import { toneOf } from './ui/tone'

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

export const TONES: GlyphTone[] = PROJECT_TONES

export const titleCase = (text: string) => text.charAt(0).toUpperCase() + text.slice(1)

/* ── small shared pieces ──────────────────────────────────────────────── */

/** One 16px folder in the project's identity tone (`ink-ghost` without one). */
export function ProjectGlyph({ color }: { color: GlyphTone | null }) {
  return <FolderGlyph color={color} />
}

/** A derived status as an outline badge: the word in ink, the tone on the dot. */
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
    <ShadcnBadge className="kb-plan-badge" title={title} variant="outline">
      <span aria-hidden="true" className="kb-plan-dot" data-tone={tone} />
      {children}
    </ShadcnBadge>
  )
}

/** Progress as a thin shadcn bar; the numbers sit beside it, not in it. */
export function ProgressBar({ done, total, label }: { done: number; total: number; label?: string }) {
  const pct = total > 0 ? Math.round((done / total) * 100) : 0
  return (
    <ShadcnProgress
      aria-label={label ?? `${done} of ${total} tickets done`}
      className="kb-plan-bar"
      data-complete={total > 0 && done >= total ? 'true' : undefined}
      value={pct}
    />
  )
}

/** The column counts of a release or a project, one compact stat per board column. */
export function ColumnTiles({
  columns,
  doneStatus,
  canceledStatus,
  counts,
  extra,
}: {
  columns: Column[]
  doneStatus: string
  canceledStatus: string | null
  counts: ColumnCounts
  extra?: ReactNode
}) {
  return (
    <div aria-label="Tickets by status" className="kb-plan-tiles" role="group">
      {columns.map((column) => (
        <div className="kb-plan-tile" data-zero={(counts[column.id] ?? 0) === 0 ? 'true' : undefined} key={column.id}>
          <span className="kb-plan-tile__label">
            <ColumnStatusIcon canceledStatus={canceledStatus} columns={columns} doneStatus={doneStatus} status={column.id} />
            {column.label}
          </span>
          <span className="kb-plan-tile__n">{counts[column.id] ?? 0}</span>
        </div>
      ))}
      {extra}
    </div>
  )
}

/** The top of a page that shows one record: the way back, then where you are (`Releases › Phase 2`). */
export function DrillBar({ root, name, onBack }: { root: string; name: string; onBack: () => void }) {
  return <Crumbs name={name} onBack={onBack} root={root} />
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
    <div className="kb-plan-edit" data-multiline={multiline ? 'true' : undefined}>
      {multiline ? (
        <ShadcnTextarea
          {...common}
          className="kb-plan-edit__area"
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
        <ShadcnInput
          {...common}
          className={mono ? 'kb-plan-edit__input kb-plan-mono' : 'kb-plan-edit__input'}
          onBlur={() => {
            if (!skipBlur.current) void commit()
          }}
          onChange={(event) => setDraft(event.target.value)}
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
        <div className="kb-plan-edit__foot">
          <ShadcnButton onClick={cancel} size="sm" type="button" variant="ghost">
            Cancel
          </ShadcnButton>
          <ShadcnButton disabled={busy} onClick={() => void commit()} size="sm" type="button">
            Save
          </ShadcnButton>
        </div>
      ) : null}
      {error ? (
        <p className="kb-field__error" id={errorId} role="alert">
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
  const uid = useId()

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
    <ShadcnDialog onOpenChange={onOpenChange} open={open}>
      <ShadcnDialogContent className="kb-dialog kb-plan-dialog">
        <ShadcnDialogHeader>
          <ShadcnDialogTitle>New release</ShadcnDialogTitle>
          <ShadcnDialogDescription>Group tickets into a delivery with a target date.</ShadcnDialogDescription>
        </ShadcnDialogHeader>
        <div className="kb-plan-form" onKeyDown={submitOnEnter(() => void submit())}>
          <div className="kb-plan-pair">
            <Field error={nameError} htmlFor={`${uid}-name`} label="Name">
              <ShadcnInput
                aria-invalid={nameError ? true : undefined}
                autoFocus
                id={`${uid}-name`}
                onChange={(event) => {
                  setName(event.target.value)
                  setError(null)
                }}
                placeholder="Phase 3"
                value={name}
              />
            </Field>
            <Field error={targetProblem} hint="Optional" htmlFor={`${uid}-target`} label="Target date">
              <ShadcnInput
                aria-invalid={targetProblem ? true : undefined}
                className="kb-plan-mono"
                id={`${uid}-target`}
                onChange={(event) => setTarget(event.target.value)}
                placeholder="YYYY-MM-DD"
                value={target}
              />
            </Field>
          </div>
          <Field hint="Optional" htmlFor={`${uid}-description`} label="Description">
            <ShadcnTextarea
              id={`${uid}-description`}
              onChange={(event) => setDescription(event.target.value)}
              placeholder="What this release delivers"
              rows={3}
              value={description}
            />
          </Field>
          {error && !nameError ? <p className="kb-field__error">{error}</p> : null}
        </div>
        <ShadcnDialogFooter>
          <ShadcnButton onClick={() => onOpenChange(false)} type="button" variant="outline">
            Cancel
          </ShadcnButton>
          <ShadcnButton disabled={!canSubmit} onClick={() => void submit()} type="button">
            {busy ? 'Creating…' : 'Create release'}
          </ShadcnButton>
        </ShadcnDialogFooter>
      </ShadcnDialogContent>
    </ShadcnDialog>
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
    <ShadcnDialog onOpenChange={onOpenChange} open={open}>
      <ShadcnDialogContent className="kb-dialog kb-plan-dialog">
        <ShadcnDialogHeader>
          <ShadcnDialogTitle>New project</ShadcnDialogTitle>
          <ShadcnDialogDescription>A ticket belongs to at most one project.</ShadcnDialogDescription>
        </ShadcnDialogHeader>
        <div className="kb-plan-form" onKeyDown={submitOnEnter(() => void submit())}>
          <Field error={nameError} hint="Unique on this board. Case is ignored." htmlFor={`${colorLabel}-name`} label="Name">
            <ShadcnInput
              aria-invalid={nameError ? true : undefined}
              autoFocus
              id={`${colorLabel}-name`}
              onChange={(event) => {
                setName(event.target.value)
                setError(null)
              }}
              placeholder="iii cloud"
              value={name}
            />
          </Field>
          <Field hint="Optional" htmlFor={`${colorLabel}-description`} label="Description">
            <ShadcnTextarea
              id={`${colorLabel}-description`}
              onChange={(event) => setDescription(event.target.value)}
              placeholder="What is this project for?"
              rows={3}
              value={description}
            />
          </Field>
          <div className="kb-plan-pair">
            <Field label="Lead">
              <LeadPicker agents={agents} onChange={(next) => setLead(next ?? undefined)} value={lead ?? null} />
            </Field>
            <div className="kb-field">
              <span className="kb-field__label" id={colorLabel}>
                Color
              </span>
              <div aria-labelledby={colorLabel} className="kb-plan-swatches" role="radiogroup">
                {TONES.map((tone) => (
                  <button
                    aria-checked={color === tone}
                    aria-label={titleCase(tone)}
                    className="kb-plan-swatch"
                    data-tone={tone}
                    key={tone}
                    onClick={() => setColor(tone)}
                    role="radio"
                    type="button"
                  >
                    <span aria-hidden="true" className="kb-plan-swatch__dot" />
                  </button>
                ))}
              </div>
            </div>
          </div>
          {error && !nameError ? <p className="kb-field__error">{error}</p> : null}
        </div>
        <ShadcnDialogFooter>
          <ShadcnButton onClick={() => onOpenChange(false)} type="button" variant="outline">
            Cancel
          </ShadcnButton>
          <ShadcnButton disabled={!canSubmit} onClick={() => void submit()} type="button">
            {busy ? 'Creating…' : 'Create project'}
          </ShadcnButton>
        </ShadcnDialogFooter>
      </ShadcnDialogContent>
    </ShadcnDialog>
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
    <ShadcnDialog onOpenChange={(next_) => !busy && onOpenChange(next_)} open={open}>
      <ShadcnDialogContent className="kb-dialog kb-plan-dialog">
        <ShadcnDialogHeader>
          <ShadcnDialogTitle>Mark {release.name} as released?</ShadcnDialogTitle>
          <ShadcnDialogDescription>
            {unfinished > 0
              ? 'This closes the release. Choose where its unfinished tickets go.'
              : 'This closes the release.'}
          </ShadcnDialogDescription>
        </ShadcnDialogHeader>
        <div className="kb-plan-form">
          <ul aria-label="What happens to the tickets" className="kb-plan-facts">
            {liveCount(release) === 0 ? (
              <li>
                <Info size={16} />
                <span>{release.name} has no tickets.</span>
              </li>
            ) : (
              <>
                <li>
                  <span className="kb-plan-dot" data-tone="ok" />
                  <span>
                    <span className="kb-plan-num-inline">{release.done}</span> done{' '}
                    {release.done === 1 ? 'ticket stays' : 'tickets stay'} in {release.name}
                  </span>
                </li>
                {unfinished > 0 ? (
                  <li>
                    <span className="kb-plan-dot" data-tone="ghost" />
                    <span>
                      <span className="kb-plan-num-inline">{unfinished}</span> {unfinished === 1 ? 'is' : 'are'} not done
                      and must go somewhere
                    </span>
                  </li>
                ) : null}
              </>
            )}
          </ul>

          {unfinished > 0 ? (
            <fieldset className="kb-plan-choices">
              <legend className="kb-field__label">Where do they go?</legend>
              <label className="kb-plan-choice" data-on={dest === 'move' ? 'true' : undefined}>
                {/* One choice, drawn as a checkbox (the board's one selection control): ticking one unticks the other. */}
                <ShadcnCheckbox
                  checked={dest === 'move'}
                  className="kb-plan-choice__box"
                  disabled={targets.length === 0}
                  onCheckedChange={(next) => {
                    if (next) setDest('move')
                  }}
                />
                <span className="kb-plan-choice__body">
                  <span className="kb-plan-choice__line">Move them to another release</span>
                  {targets.length > 0 ? (
                    <SearchPicker
                      label="Release to move the tickets to"
                      onChange={(value) => {
                        setTargetId(value)
                        setDest('move')
                      }}
                      options={targets.map((candidate) => ({
                        value: candidate.id,
                        label: candidate.name,
                        description: candidate.target_at ? `Target ${formatTarget(candidate.target_at)}` : undefined,
                        icon: <Milestone size={16} />,
                      }))}
                      searchPlaceholder="Find a release…"
                      trigger={
                        <PickerButton icon={<Milestone size={16} />} placeholder={!target}>
                          {target?.name ?? 'Pick a release'}
                        </PickerButton>
                      }
                      value={targetId ?? null}
                    />
                  ) : null}
                  <span className="kb-field__hint">
                    {target
                      ? `${target.id === next?.id && isNextByDate(target, release) ? `${target.name} is the next release by target date. ` : ''}It will hold ${plural(holds, 'ticket')}.`
                      : targets.length > 0
                        ? 'Pick the release they move to.'
                        : 'There is no other release that is not released yet.'}
                  </span>
                </span>
              </label>
              <label className="kb-plan-choice" data-on={dest === 'remove' ? 'true' : undefined}>
                <ShadcnCheckbox
                  checked={dest === 'remove'}
                  className="kb-plan-choice__box"
                  onCheckedChange={(next) => {
                    if (next) setDest('remove')
                  }}
                />
                <span className="kb-plan-choice__body">
                  <span className="kb-plan-choice__line">Remove them from any release</span>
                  <span className="kb-field__hint">They stay on the board with no release.</span>
                </span>
              </label>
            </fieldset>
          ) : null}

          <ul className="kb-plan-notes">
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
          {error ? <p className="kb-field__error">{error}</p> : null}
        </div>
        <ShadcnDialogFooter>
          {/* A ConfirmDialog cannot hold the radios; this one gives Cancel the focus the way it does. */}
          <ShadcnButton autoFocus disabled={busy} onClick={() => onOpenChange(false)} type="button" variant="outline">
            Cancel
          </ShadcnButton>
          <ShadcnButton disabled={busy || blocked} onClick={() => void submit()} type="button">
            {busy ? 'Marking…' : 'Mark as released'}
          </ShadcnButton>
        </ShadcnDialogFooter>
      </ShadcnDialogContent>
    </ShadcnDialog>
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
            {
              total: own.length,
              done: own.filter((ticket) => ticket.status === data.done_status).length,
              canceled: own.filter((ticket) => ticket.status === data.canceled_status).length,
            },
          ]
        }),
      ),
    [tickets, label, family, data.done_status, data.canceled_status],
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
    return own && allClosed(own)
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

  const labelOptions: PickerOption[] = labels.map((entry) => ({
    value: entry.label,
    label: entry.label,
    count: entry.count,
    icon: <span aria-hidden="true" className="kb-label__dot" data-tone={toneOf(entry.label)} />,
  }))
  const entityOptions: PickerOption[] = entities.map((entity) => ({
    value: entity.id,
    label: entity.name,
    count: liveCount(entity),
    icon: into === 'release' ? <Milestone size={16} /> : <FolderGlyph color={(entity as ProjectView).color ?? null} />,
  }))

  return (
    <ShadcnDialog onOpenChange={(next) => !busy && onOpenChange(next)} open={open}>
      <ShadcnDialogContent className="kb-dialog kb-plan-dialog kb-plan-dialog--wide">
        <ShadcnDialogHeader>
          <ShadcnDialogTitle>Create from a label</ShadcnDialogTitle>
          <ShadcnDialogDescription>
            Puts the tickets that carry a label into a {noun}. Nothing is deleted.
          </ShadcnDialogDescription>
        </ShadcnDialogHeader>
        <div className="kb-plan-form kb-plan-form--scroll">
          <div className="kb-plan-pair">
            <Field label="Label" hint={chosenLabel ? plural(chosenLabel.count, 'ticket') : undefined}>
              <SearchPicker
                empty="No ticket carries a label yet."
                label="Label"
                onChange={chooseLabel}
                options={labelOptions}
                searchPlaceholder="Find a label…"
                trigger={
                  <PickerButton
                    icon={label ? <span aria-hidden="true" className="kb-label__dot" data-tone={toneOf(label)} /> : <Tag size={16} />}
                    placeholder={!label}
                  >
                    {label ?? (labels.length === 0 ? 'No ticket has a label yet' : 'Pick a label')}
                  </PickerButton>
                }
                value={label ?? null}
              />
            </Field>
            <div className="kb-field">
              <span className="kb-field__label" id={`${uid}-into`}>
                Put them in
              </span>
              <ShadcnTabs
                onValueChange={(next) => {
                  autoExisting.current = false
                  switchMode(next === 'existing' ? 'existing' : 'new')
                }}
                value={mode}
              >
                <ShadcnTabsList aria-labelledby={`${uid}-into`} className="kb-plan-modes">
                  <ShadcnTabsTrigger value="new">New {noun}</ShadcnTabsTrigger>
                  <ShadcnTabsTrigger value="existing">Existing {noun}</ShadcnTabsTrigger>
                </ShadcnTabsList>
              </ShadcnTabs>
            </div>
            {mode === 'new' ? (
              <>
                <Field
                  hint={
                    chosenPreview && !failed(chosenPreview) && chosenPreview.entity
                      ? `A ${noun} named ${chosenPreview.entity.name} already exists. The tickets are added to it.`
                      : undefined
                  }
                  htmlFor={`${uid}-name`}
                  label={`${titleCase(noun)} name`}
                >
                  <ShadcnInput
                    disabled={!label}
                    id={`${uid}-name`}
                    onChange={(event) => {
                      const next = event.target.value
                      if (label) setNames((current) => ({ ...current, [key(label)]: next }))
                    }}
                    placeholder={label ? suggestName(label) : ''}
                    value={label ? nameOf(label) : ''}
                  />
                </Field>
                {into === 'release' ? (
                  <Field
                    error={targetProblem}
                    hint={batch && label ? `Only for ${nameOf(label)}. Set the others on their pages.` : 'Optional'}
                    htmlFor={`${uid}-target`}
                    label="Target date"
                  >
                    <ShadcnInput
                      aria-invalid={targetProblem ? true : undefined}
                      className="kb-plan-mono"
                      id={`${uid}-target`}
                      onChange={(event) => setTarget(event.target.value)}
                      placeholder="YYYY-MM-DD"
                      value={target}
                    />
                  </Field>
                ) : null}
              </>
            ) : (
              <>
                <Field
                  hint={
                    existing
                      ? `${titleCase(existing.status)}${'target_at' in existing && existing.target_at ? ` · target ${formatTarget(existing.target_at)}` : ''}`
                      : undefined
                  }
                  label={titleCase(noun)}
                >
                  <SearchPicker
                    empty={`No ${noun} yet.`}
                    label={titleCase(noun)}
                    onChange={setExistingId}
                    options={entityOptions}
                    searchPlaceholder={`Find a ${noun}…`}
                    trigger={
                      <PickerButton
                        icon={into === 'release' ? <Milestone size={16} /> : <FolderGlyph color={(existing as ProjectView | undefined)?.color ?? null} />}
                        placeholder={!existing}
                      >
                        {existing?.name ?? (entities.length === 0 ? `No ${noun} yet` : `Pick a ${noun}`)}
                      </PickerButton>
                    }
                    value={existingId ?? null}
                  />
                </Field>
              </>
            )}
          </div>

          {label ? (
            <section aria-label={`Preview for ${label}`} className="kb-plan-preview">
              <header className="kb-plan-preview__head">
                <h3 className="kb-plan-preview__title">
                  Preview for <span className="kb-plan-mono">{label}</span>
                  {heading}
                </h3>
                <span className="kb-field__hint">{labelSummary}</span>
              </header>
              {chosenPreview && failed(chosenPreview) ? (
                <p className="kb-field__error" role="alert">
                  {chosenPreview.error}
                </p>
              ) : buckets ? (
                <ul className="kb-plan-buckets">
                  <Bucket
                    n={buckets.added.length}
                    note={`Go into ${existing?.name ?? chosenName(chosenPreview, nameOf(label), noun)}`}
                    title="Will be added"
                    tone="ok"
                  />
                  <Bucket n={buckets.already_here.length} note="Left as they are" title="Already here" />
                  <Bucket
                    n={buckets.in_other.length}
                    note={
                      // biome-ignore lint/a11y/noLabelWithoutControl: the Base UI checkbox inside is the control
                      <label className="kb-plan-check" data-disabled={inOther === 0 ? 'true' : undefined}>
                        <ShadcnCheckbox checked={replace} disabled={inOther === 0} onCheckedChange={setReplace} />
                        Move them here
                      </label>
                    }
                    title={`In another ${noun}`}
                    tone={buckets.in_other.length > 0 ? 'warn' : undefined}
                  />
                  <Bucket n={buckets.deleted.length} note="Skipped" title="Deleted" />
                </ul>
              ) : (
                <ul className="kb-plan-buckets" aria-busy="true">
                  {[0, 1, 2, 3].map((index) => (
                    <li className="kb-plan-bucket" key={index}>
                      <ShadcnSkeleton className="kb-plan-skeleton kb-plan-skeleton--line" />
                    </li>
                  ))}
                </ul>
              )}
            </section>
          ) : null}

          {batch ? (
            <section aria-labelledby={`${uid}-batch`} className="kb-plan-batch">
              <header className="kb-plan-batch__head">
                <h3 className="kb-plan-preview__title" id={`${uid}-batch`}>
                  One {noun} for each matching label
                </h3>
                <span className="kb-plan-mono kb-field__hint">{stem}</span>
                <span className="kb-plan-spacer" />
                <span className="kb-field__hint">
                  {selected.length} of {tableRows.length} selected
                </span>
              </header>
              <div className="kb-plan-batch__table">
                <ShadcnTable>
                  <ShadcnTableHeader>
                    <ShadcnTableRow>
                      <ShadcnTableHead className="kb-plan-cell--check">
                        <ShadcnCheckbox
                          aria-label={`Select all ${tableRows.length} labels`}
                          checked={everyOn}
                          indeterminate={!everyOn && someOn}
                          onCheckedChange={() =>
                            setPicked(everyOn ? {} : Object.fromEntries(tableRows.map((entry) => [key(entry), true])))
                          }
                        />
                      </ShadcnTableHead>
                      <ShadcnTableHead>Label</ShadcnTableHead>
                      <ShadcnTableHead>{titleCase(noun)} name</ShadcnTableHead>
                      <ShadcnTableHead className="kb-plan-num">Tickets</ShadcnTableHead>
                      <ShadcnTableHead className="kb-plan-num">Done</ShadcnTableHead>
                      <ShadcnTableHead className="kb-plan-num">Will add</ShadcnTableHead>
                      <ShadcnTableHead className="kb-plan-num">Here</ShadcnTableHead>
                      <ShadcnTableHead className="kb-plan-num">Elsewhere</ShadcnTableHead>
                    </ShadcnTableRow>
                  </ShadcnTableHeader>
                  <ShadcnTableBody>
                    {tableRows.map((entry) => {
                      const preview = previewOf(entry)
                      const on = isOn(entry)
                      const own = stats[key(entry)]
                      return (
                        <ShadcnTableRow
                          aria-current={isChosen(entry) ? 'true' : undefined}
                          className="kb-plan-batch__row"
                          data-off={on ? undefined : 'true'}
                          key={key(entry)}
                        >
                          <ShadcnTableCell className="kb-plan-cell--check">
                            <ShadcnCheckbox
                              aria-label={`Create a ${noun} for ${entry}`}
                              checked={on}
                              disabled={isChosen(entry)}
                              onCheckedChange={(checked) => setPicked((current) => ({ ...current, [key(entry)]: checked }))}
                            />
                          </ShadcnTableCell>
                          <ShadcnTableCell className="kb-plan-mono">{entry}</ShadcnTableCell>
                          <ShadcnTableCell>
                            <ShadcnInput
                              aria-label={`${titleCase(noun)} name for ${entry}`}
                              className="kb-plan-batch__input"
                              onChange={(event) => {
                                const next = event.target.value
                                setNames((current) => ({ ...current, [key(entry)]: next }))
                              }}
                              value={nameOf(entry)}
                            />
                          </ShadcnTableCell>
                          <ShadcnTableCell className="kb-plan-num">{own?.total ?? 0}</ShadcnTableCell>
                          <ShadcnTableCell className="kb-plan-num">{own?.done ?? 0}</ShadcnTableCell>
                          <ShadcnTableCell className="kb-plan-num">
                            {preview ? willAdd(preview.buckets, replace) : '…'}
                          </ShadcnTableCell>
                          <ShadcnTableCell className="kb-plan-num">
                            {preview ? preview.buckets.already_here.length : '…'}
                          </ShadcnTableCell>
                          <ShadcnTableCell className="kb-plan-num">
                            {preview ? preview.buckets.in_other.length : '…'}
                          </ShadcnTableCell>
                        </ShadcnTableRow>
                      )
                    })}
                    <ShadcnTableRow className="kb-plan-batch__total">
                      <ShadcnTableCell />
                      <ShadcnTableCell>Total</ShadcnTableCell>
                      <ShadcnTableCell>{plural(selected.length, noun)}</ShadcnTableCell>
                      <ShadcnTableCell className="kb-plan-num">{labelTicketCount}</ShadcnTableCell>
                      <ShadcnTableCell className="kb-plan-num">
                        {selected.reduce((sum, entry) => sum + (stats[key(entry)]?.done ?? 0), 0)}
                      </ShadcnTableCell>
                      <ShadcnTableCell className="kb-plan-num">{sumAdd}</ShadcnTableCell>
                      <ShadcnTableCell className="kb-plan-num">
                        {selected.reduce((sum, entry) => sum + (previewOf(entry)?.buckets.already_here.length ?? 0), 0)}
                      </ShadcnTableCell>
                      <ShadcnTableCell className="kb-plan-num">{inOther}</ShadcnTableCell>
                    </ShadcnTableRow>
                  </ShadcnTableBody>
                </ShadcnTable>
              </div>
            </section>
          ) : null}

          {label ? (
            <div className="kb-plan-checks">
              {into === 'release' ? (
                // biome-ignore lint/a11y/noLabelWithoutControl: the Base UI checkbox inside is the control
                <label className="kb-plan-check" data-disabled={markDisabled ? 'true' : undefined}>
                  <ShadcnCheckbox checked={markNow} disabled={markDisabled} onCheckedChange={setMarkReleased} />
                  <span className="kb-plan-check__text">
                    <span>Mark releases whose tickets are all done as Released</span>
                    <span className="kb-field__hint">
                      {mode === 'existing'
                        ? `When every ticket of ${existing?.name ?? 'the release'} is done`
                        : releasable.length > 0
                          ? `${plural(releasable.length, 'release')}: ${nameList(releasable.map(nameOf))}`
                          : 'No selected label has all its tickets done'}
                    </span>
                  </span>
                </label>
              ) : null}
              {/* biome-ignore lint/a11y/noLabelWithoutControl: the Base UI checkbox inside is the control */}
              <label className="kb-plan-check">
                <ShadcnCheckbox checked={removeLabel} onCheckedChange={setRemoveLabel} />
                <span className="kb-plan-check__text">
                  <span>Remove the label from these tickets</span>
                  <span className="kb-field__hint">
                    {removeLabel
                      ? `Off the tickets that end up in the ${noun}`
                      : `Off: ${selected.length > 1 ? `${selected[0]} … ${selected[selected.length - 1]} stay` : `${selected[0] ?? label} stays`} on all ${labelTicketCount} ${labelTicketCount === 1 ? 'ticket' : 'tickets'}`}
                  </span>
                </span>
              </label>
            </div>
          ) : null}

          {error ? (
            <p className="kb-field__error" role="alert">
              {error}
            </p>
          ) : null}
        </div>
        <ShadcnDialogFooter className="kb-plan-dialog__foot">
          {reason && label ? (
            <span className="kb-plan-reason" id={`${uid}-why`}>
              <CircleAlert size={16} />
              {reason}
            </span>
          ) : null}
          <ShadcnButton disabled={busy} onClick={() => onOpenChange(false)} type="button" variant="outline">
            Cancel
          </ShadcnButton>
          <ShadcnButton
            aria-describedby={reason ? `${uid}-why` : undefined}
            disabled={!canRun}
            onClick={() => void run()}
            type="button"
          >
            {busy ? 'Working…' : confirmLabel}
          </ShadcnButton>
        </ShadcnDialogFooter>
      </ShadcnDialogContent>
    </ShadcnDialog>
  )
}

function chosenName(preview: Preview | undefined, fallback: string, noun: string): string {
  if (preview && !failed(preview) && preview.entity) return preview.entity.name
  return fallback.trim() || `the ${noun}`
}

/** One preview bucket: the count, what it means, and what happens to those tickets. */
function Bucket({ n, title, note, tone }: { n: number; title: string; note: ReactNode; tone?: 'ok' | 'warn' }) {
  return (
    <li className="kb-plan-bucket" data-zero={n === 0 ? 'true' : undefined}>
      <span className="kb-plan-bucket__n" data-tone={n > 0 ? tone : undefined}>
        {n}
      </span>
      <span className="kb-plan-bucket__title">{title}</span>
      <span className="kb-plan-bucket__note">{note}</span>
    </li>
  )
}

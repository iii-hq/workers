import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import { ProjectDialog } from '@/components/projects/ProjectDialog'
import { ProjectDot } from '@/components/projects/ProjectDot'
import { Button } from '@/components/shadcn/button'
import { Dialog } from '@/components/shadcn/dialog'
import { Switch } from '@/components/shadcn/switch'
import {
  PanelDialogContent,
  PanelDialogHeader,
  PanelDialogTitle,
} from '@/components/ui/FormDialog'
import {
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/Table'
import { TickSlider } from '@/components/ui/TickSlider'
import { useChatProjects } from '@/hooks/use-chat-projects'
import { useChatSettings } from '@/hooks/use-chat-settings'
import { MOBILE_LAYOUT_QUERY, useMediaQuery } from '@/hooks/use-media-query'
import {
  AUTOCOMPACT_DEFAULT_TOKENS,
  AUTOCOMPACT_INSTRUCTIONS_MAX,
  AUTOCOMPACT_MIN_TOKENS,
  AUTOCOMPACT_SLIDER_MAX,
  AUTOCOMPACT_SLIDER_STEP,
  autocompactOf,
  COLUMN_EFFECTIVE,
  COLUMN_LIMIT,
  COLUMN_WHERE,
  effectiveCell,
  formatTokens,
  type GlobalAutocompact,
  globalTokens,
  HISTORY_EMPTY,
  HISTORY_HEADING,
  historyChange,
  INHERIT,
  limitCell,
  MODELS_EMPTY,
  MODELS_HEADING,
  type ModelUse,
  modelNumbers,
  modelOutcome,
  OVERRIDES_HEADING,
  OVERRIDES_HINT,
  parseAutocompactSetting,
  parseTokens,
  resolveAutocompact,
  SETTINGS_HINT,
  SETTINGS_INTRO,
  SETTINGS_TITLE,
  SUMMARY_HINT,
  SUMMARY_LABEL,
  SUMMARY_PLACEHOLDER,
  SWITCH_LABEL,
  sessionsCount,
  sessionWhere,
  VALUE_LEAD,
  VALUE_TRAIL,
} from '@/lib/autocompact'
import { getDefaultBackend } from '@/lib/backend'
import {
  type AutocompactLogEntry,
  type AutocompactModel,
  listAutocompactLog,
  listAutocompactModels,
} from '@/lib/backend/autocompact'
import {
  autocompactRejection,
  type ChatProject,
  NO_PROJECT,
  parseProjectError,
} from '@/lib/backend/projects'
import { isSideChat } from '@/lib/chat-aside'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { formatRelative } from '@/lib/relative-age'
import {
  sessionListHasMore,
  subscribeSessionListHasMore,
} from '@/lib/sessions/api'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'
import { AutocompactTokenInput } from './AutocompactTokenInput'
import { ConversationDetailsDialog } from './ConversationDetailsDialog'

/** The two harness reads behind the page; tests and stories replace them. */
export interface AutocompactSource {
  models: () => Promise<AutocompactModel[]>
  log: () => Promise<AutocompactLogEntry[]>
}

const ENGINE_SOURCE: AutocompactSource = {
  models: () => listAutocompactModels(),
  log: () => listAutocompactLog(),
}

interface Loaded<T> {
  rows: T[] | null
  error: string | null
}

function errText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

const FIELD =
  'w-full rounded-lg bg-muted px-3 text-sm text-foreground ring-1 ring-transparent outline-none transition-shadow duration-(--motion-duration-control) placeholder:text-ink-ghost focus-visible:ring-ring/50 aria-invalid:ring-alert/60'

/** Where a typed rejection's words go (D167). */
type ErrorField = 'value' | 'summary' | 'form'

function rejection(err: unknown): { field: ErrorField; message: string } {
  const parsed = parseProjectError(err)
  const code = parsed?.code ?? ''
  const message =
    parsed?.message ??
    (code.startsWith('bad_') ? autocompactRejection(code) : errText(err))
  if (code === 'bad_tokens') return { field: 'value', message }
  if (code === 'bad_instructions') return { field: 'summary', message }
  return { field: 'form', message: `Not saved: ${message}` }
}

/** The whole global setting with a change; `instructions: ""` clears them. */
function withChange(
  global: GlobalAutocompact,
  change: Partial<GlobalAutocompact>,
): GlobalAutocompact {
  const merged = { ...global, ...change }
  return {
    mode: merged.mode,
    ...(merged.tokens !== undefined ? { tokens: merged.tokens } : {}),
    ...(merged.instructions !== undefined
      ? { instructions: merged.instructions }
      : {}),
  }
}

interface OverrideRow {
  key: string
  project: ChatProject | null
  conversation: Conversation | null
  where: string
  limit: string
  effective: string
}

function overrideRows(
  projects: readonly ChatProject[],
  conversations: readonly Conversation[],
  global: GlobalAutocompact,
  models: readonly ModelUse[],
): { rows: OverrideRow[]; sessions: number } {
  const rows: OverrideRow[] = projects.map((project) => {
    const own = resolveAutocompact({ session: INHERIT, project, global })
    return {
      key: `project:${project.id}`,
      project,
      conversation: null,
      where: project.name,
      limit: limitCell(parseAutocompactSetting(project.autocompact), global),
      effective: effectiveCell(own.tokens, models),
    }
  })
  // Sessions with a value of their own, from the loaded list (D173). A side
  // chat follows its conversation, whatever its frozen copy says.
  const sessions = conversations.filter(
    (c) => !c.draft && !isSideChat(c) && autocompactOf(c).mode !== 'inherit',
  )
  for (const conversation of sessions) {
    const own = autocompactOf(conversation)
    const project =
      conversation.projectId && conversation.projectId !== NO_PROJECT
        ? (projects.find((p) => p.id === conversation.projectId) ?? null)
        : null
    rows.push({
      key: `session:${conversation.id}`,
      project: null,
      conversation,
      where: sessionWhere(conversation.title, project?.name ?? null),
      limit: limitCell(own, global),
      effective: effectiveCell(
        own.mode === 'tokens' ? own.tokens : null,
        models,
      ),
    })
  }
  return { rows, sessions: sessions.length }
}

/**
 * Settings › Chat › Auto-compact (chat-projects §3.3.1, D170): the global
 * setting in a `PanelDialog`, as Claude Code accounts. The switch, the size
 * (a field to 2M and a slider to 1M with a grey tick at each model's limit),
 * the models used here in the last 7 days, the summary's instructions, the
 * projects and sessions with a value of their own, and the last
 * auto-compactions. Every change writes `harness::chat-settings::set` at
 * once; a typed rejection shows under its field.
 */
export function AutocompactDialog({
  open,
  onOpenChange,
  source = ENGINE_SOURCE,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  source?: AutocompactSource
}) {
  const ids = useId()
  const realBackend = getDefaultBackend().id === 'real'
  const chatSettings = useChatSettings(realBackend && open)
  const chatProjects = useChatProjects(realBackend && open)
  const ctx = useConversationsCtxOptional()
  const narrow = useMediaQuery(MOBILE_LAYOUT_QUERY)
  const more = useSyncExternalStore(
    subscribeSessionListHasMore,
    sessionListHasMore,
    sessionListHasMore,
  )
  const global = chatSettings.autocompact
  const tokens = globalTokens(global)
  const on = tokens !== null
  /** The last size set, kept while off so turning on restores it. */
  const lastTokens = global.tokens ?? AUTOCOMPACT_DEFAULT_TOKENS

  const [models, setModels] = useState<Loaded<AutocompactModel>>({
    rows: null,
    error: null,
  })
  const [log, setLog] = useState<Loaded<AutocompactLogEntry>>({
    rows: null,
    error: null,
  })
  // A fresh read on every opening: the models and the log move with turns.
  useEffect(() => {
    if (!open) return
    let alive = true
    const read = <T,>(
      load: () => Promise<T[]>,
      set: (next: Loaded<T>) => void,
    ) =>
      load()
        .then((rows) => {
          if (alive) set({ rows, error: null })
        })
        .catch((err: unknown) => {
          if (alive) set({ rows: null, error: errText(err) })
        })
    void read(source.models, setModels)
    void read(source.log, setLog)
    return () => {
      alive = false
    }
  }, [open, source])

  const [text, setText] = useState(formatTokens(lastTokens))
  const [dragging, setDragging] = useState<number | null>(null)
  const [instructions, setInstructions] = useState(global.instructions ?? '')
  const [errors, setErrors] = useState<Partial<Record<ErrorField, string>>>({})
  const fieldRef = useRef<HTMLInputElement>(null)
  const summaryRef = useRef<HTMLTextAreaElement>(null)

  // What the person typed stays until it is saved: another device's change
  // and a refused write (whose optimistic value rolls back) show only in a
  // field they have not touched since the last save.
  const textOwned = useRef(false)
  const summaryOwned = useRef(false)
  useEffect(() => {
    if (!textOwned.current) setText(formatTokens(lastTokens))
  }, [lastTokens])
  const savedInstructions = global.instructions ?? ''
  useEffect(() => {
    if (!summaryOwned.current) setInstructions(savedInstructions)
  }, [savedInstructions])
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-seeded per open, not on every live update
  useEffect(() => {
    if (!open) return
    textOwned.current = false
    summaryOwned.current = false
    setText(formatTokens(lastTokens))
    setInstructions(savedInstructions)
    setErrors({})
    setDragging(null)
  }, [open])

  /** Write the whole setting with `change`; a refusal shows under its field. */
  const write = async (
    change: Partial<GlobalAutocompact>,
    owner: 'value' | 'summary',
  ) => {
    setErrors({})
    try {
      await chatSettings.setAutocompact(withChange(global, change))
      if (owner === 'value') textOwned.current = false
      else summaryOwned.current = false
    } catch (err) {
      const shown = rejection(err)
      setErrors({ [shown.field]: shown.message })
      if (shown.field === 'value') fieldRef.current?.focus()
      if (shown.field === 'summary') summaryRef.current?.focus()
    }
  }

  const commitTokens = (next: number) => {
    if (next === global.tokens && on) return
    void write({ mode: 'tokens', tokens: next }, 'value')
  }

  const commitField = () => {
    const parsed = parseTokens(text)
    if (!parsed.ok) {
      setErrors({ value: parsed.error })
      return
    }
    setErrors((current) => ({ ...current, value: undefined }))
    setText(formatTokens(parsed.tokens))
    commitTokens(parsed.tokens)
  }

  const modelUses: ModelUse[] = useMemo(
    () => (models.rows ?? []).map((m) => ({ name: m.name, usable: m.usable })),
    [models.rows],
  )
  const overrides = useMemo(
    () =>
      overrideRows(
        chatProjects.projects,
        ctx?.conversations ?? [],
        global,
        modelUses,
      ),
    [chatProjects.projects, ctx?.conversations, global, modelUses],
  )

  const [editingProject, setEditingProject] = useState<ChatProject | null>(null)
  const [editingConversation, setEditingConversation] =
    useState<Conversation | null>(null)
  const openRow = (row: OverrideRow) => {
    if (row.project) setEditingProject(row.project)
    else if (row.conversation && ctx) setEditingConversation(row.conversation)
  }
  const rowOpens = (row: OverrideRow) =>
    Boolean(row.project || (row.conversation && ctx))

  const sliderValue = Math.min(
    AUTOCOMPACT_SLIDER_MAX,
    dragging ?? tokens ?? lastTokens,
  )
  const instructionsDirty = instructions !== savedInstructions

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <PanelDialogContent
          data-autocompact-dialog=""
          className="flex max-h-[calc(100dvh-2rem)] w-[min(48rem,calc(100vw-2rem))] flex-col [--dialog-max-width:48rem]"
        >
          <PanelDialogHeader>
            <PanelDialogTitle>{SETTINGS_TITLE}</PanelDialogTitle>
          </PanelDialogHeader>
          <div className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto p-5">
            <p className="text-body text-ink-faint">{SETTINGS_INTRO}</p>
            {errors.form ? (
              <p role="alert" className="text-sm text-alert">
                {errors.form}
              </p>
            ) : null}

            <section
              className="flex flex-col gap-3"
              aria-label={SETTINGS_TITLE}
            >
              <div className="flex items-center gap-3">
                <label
                  htmlFor={`${ids}-switch`}
                  className="flex-1 text-sm font-medium text-ink"
                >
                  {SWITCH_LABEL}
                </label>
                <Switch
                  id={`${ids}-switch`}
                  aria-label={SWITCH_LABEL}
                  checked={on}
                  onCheckedChange={(checked) => {
                    const parsed = parseTokens(text)
                    // A size that isn't one goes back to the saved size:
                    // the field never shows a value the setting doesn't hold.
                    if (!parsed.ok) {
                      textOwned.current = false
                      setText(formatTokens(lastTokens))
                    }
                    void write(
                      checked === true
                        ? {
                            mode: 'tokens',
                            tokens: parsed.ok ? parsed.tokens : lastTokens,
                          }
                        : {
                            mode: 'off',
                            tokens: parsed.ok ? parsed.tokens : lastTokens,
                          },
                      'value',
                    )
                  }}
                />
              </div>
              <div className="flex flex-wrap items-center gap-2 text-sm text-ink">
                <label htmlFor={`${ids}-tokens`}>{VALUE_LEAD}</label>
                <AutocompactTokenInput
                  ref={fieldRef}
                  id={`${ids}-tokens`}
                  name="autocompact-tokens"
                  value={text}
                  disabled={!on}
                  onChange={(next) => {
                    textOwned.current = true
                    setText(next)
                    if (errors.value) {
                      setErrors((c) => ({ ...c, value: undefined }))
                    }
                  }}
                  onCommit={commitField}
                  invalid={Boolean(errors.value)}
                  aria-describedby={
                    errors.value ? `${ids}-value-error` : `${ids}-hint`
                  }
                />
                <span>{VALUE_TRAIL}</span>
              </div>
              {errors.value ? (
                <p
                  id={`${ids}-value-error`}
                  role="alert"
                  className="text-xs text-alert"
                >
                  {errors.value}
                </p>
              ) : null}
              <div className="flex flex-col gap-1 px-1.5">
                <TickSlider
                  value={sliderValue}
                  min={0}
                  max={AUTOCOMPACT_SLIDER_MAX}
                  step={AUTOCOMPACT_SLIDER_STEP}
                  disabled={!on}
                  label={`${VALUE_LEAD} (${VALUE_TRAIL})`}
                  valueText={(v) => `${formatTokens(v)} tokens`}
                  ticks={(models.rows ?? []).map((m) => ({
                    value: m.usable,
                    label: `${m.name}: ${modelNumbers(m.context_window, m.usable)}`,
                  }))}
                  onValueChange={(v) => {
                    const next = Math.max(AUTOCOMPACT_MIN_TOKENS, v)
                    textOwned.current = true
                    setDragging(next)
                    setText(formatTokens(next))
                    setErrors((c) => ({ ...c, value: undefined }))
                  }}
                  onValueCommitted={(v) => {
                    setDragging(null)
                    commitTokens(Math.max(AUTOCOMPACT_MIN_TOKENS, v))
                  }}
                />
                <div className="flex justify-between font-mono text-meta text-ink-faint tabular-nums">
                  <span>0</span>
                  <span>{formatTokens(AUTOCOMPACT_SLIDER_MAX / 2)}</span>
                  <span>{formatTokens(AUTOCOMPACT_SLIDER_MAX)}</span>
                </div>
              </div>
              <p id={`${ids}-hint`} className="text-xs text-ink-faint">
                {SETTINGS_HINT}
              </p>
            </section>

            <section
              className="flex flex-col gap-2"
              aria-labelledby={`${ids}-models`}
            >
              <h3
                id={`${ids}-models`}
                className="text-sm font-semibold text-ink"
              >
                {MODELS_HEADING}
              </h3>
              {models.error ? (
                <p role="alert" className="text-xs text-alert">
                  {models.error}
                </p>
              ) : models.rows === null ? (
                <p className="text-xs text-ink-faint">Reading…</p>
              ) : models.rows.length === 0 ? (
                <p className="text-xs text-ink-faint">{MODELS_EMPTY}</p>
              ) : (
                <ul className="flex flex-col divide-y divide-border rounded-lg border border-border">
                  {models.rows.map((m) => (
                    <li
                      key={`${m.provider}:${m.model}`}
                      data-autocompact-model={m.model}
                      className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5 px-3 py-2"
                    >
                      <span className="min-w-24 flex-1 truncate text-sm text-ink">
                        {m.name}
                      </span>
                      <span className="font-mono text-xs text-ink-faint tabular-nums">
                        {modelNumbers(m.context_window, m.usable)}
                      </span>
                      <span
                        data-autocompact-outcome=""
                        className={cn(
                          'text-xs',
                          on && tokens !== null && tokens < m.usable
                            ? 'text-ink'
                            : 'text-ink-faint',
                        )}
                      >
                        {modelOutcome(tokens, m.usable)}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section className="flex flex-col gap-1.5">
              <div className="flex items-center gap-2">
                <label
                  htmlFor={`${ids}-summary`}
                  className="flex-1 text-sm font-semibold text-ink"
                >
                  {SUMMARY_LABEL}
                </label>
                <span
                  className="font-mono text-meta text-ink-faint tabular-nums"
                  aria-live="polite"
                >
                  {instructions.length}/{AUTOCOMPACT_INSTRUCTIONS_MAX}
                </span>
              </div>
              <textarea
                ref={summaryRef}
                id={`${ids}-summary`}
                name="autocompact-instructions"
                rows={3}
                value={instructions}
                maxLength={AUTOCOMPACT_INSTRUCTIONS_MAX}
                placeholder={SUMMARY_PLACEHOLDER}
                aria-invalid={errors.summary ? true : undefined}
                aria-describedby={
                  errors.summary
                    ? `${ids}-summary-error`
                    : `${ids}-summary-hint`
                }
                onChange={(e) => {
                  summaryOwned.current = true
                  setInstructions(e.currentTarget.value)
                  if (errors.summary) {
                    setErrors((c) => ({ ...c, summary: undefined }))
                  }
                }}
                className={cn(FIELD, 'resize-y py-2 leading-5')}
              />
              <div className="flex flex-wrap items-center gap-3">
                {errors.summary ? (
                  <p
                    id={`${ids}-summary-error`}
                    role="alert"
                    className="flex-1 text-xs text-alert"
                  >
                    {errors.summary}
                  </p>
                ) : (
                  <p
                    id={`${ids}-summary-hint`}
                    className="flex-1 text-xs text-ink-faint"
                  >
                    {SUMMARY_HINT}
                  </p>
                )}
                {instructionsDirty ? (
                  <Button
                    type="button"
                    size="sm"
                    variant="secondary"
                    onClick={() => {
                      const next = instructions.trim()
                      setInstructions(next)
                      void write({ instructions: next }, 'summary')
                    }}
                  >
                    Save
                  </Button>
                ) : null}
              </div>
            </section>

            <section
              className="flex flex-col gap-3"
              aria-labelledby={`${ids}-overrides`}
            >
              <div className="flex flex-col gap-1">
                <h3
                  id={`${ids}-overrides`}
                  className="text-sm font-semibold text-ink"
                >
                  {OVERRIDES_HEADING}
                </h3>
                <p className="text-xs text-ink-faint">{OVERRIDES_HINT}</p>
                {overrides.sessions > 0 || more ? (
                  <p
                    data-autocompact-sessions=""
                    className="text-xs text-ink-faint"
                  >
                    {sessionsCount(overrides.sessions, more)}
                  </p>
                ) : null}
              </div>
              {overrides.rows.length === 0 ? null : narrow ? (
                <ul className="flex flex-col gap-2">
                  {overrides.rows.map((row) => (
                    <li key={row.key}>
                      <button
                        type="button"
                        data-autocompact-override={row.key}
                        disabled={!rowOpens(row)}
                        onClick={() => openRow(row)}
                        className="flex w-full flex-col gap-1 rounded-lg border border-border px-3 py-2.5 text-left text-xs text-ink-faint enabled:hover:bg-surface-hover focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50"
                      >
                        <span className="flex items-center gap-2 text-sm font-medium text-ink">
                          {row.project ? (
                            <ProjectDot color={row.project.color} />
                          ) : null}
                          <span className="min-w-0 flex-1 truncate">
                            {row.where}
                          </span>
                          <span className="font-mono text-xs font-normal text-ink-faint">
                            {row.limit}
                          </span>
                        </span>
                        <span>{row.effective}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              ) : (
                <TableFrame>
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>{COLUMN_WHERE}</TableHead>
                        <TableHead>{COLUMN_LIMIT}</TableHead>
                        <TableHead>{COLUMN_EFFECTIVE}</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {overrides.rows.map((row) => (
                        <TableRow
                          key={row.key}
                          data-autocompact-override={row.key}
                          interactive={rowOpens(row)}
                          aria-label={
                            rowOpens(row)
                              ? `Edit ${row.project?.name ?? row.conversation?.title}'s auto-compact`
                              : undefined
                          }
                          onClick={
                            rowOpens(row) ? () => openRow(row) : undefined
                          }
                        >
                          {/* Where and Effective wrap inside the dialog's
                              width (the cells default to one line, and the
                              frame clips); the limit stays on one line. */}
                          <TableCell>
                            <span className="flex min-w-32 items-start gap-2 font-medium whitespace-normal text-ink">
                              {row.project ? (
                                <ProjectDot
                                  color={row.project.color}
                                  className="mt-1.5"
                                />
                              ) : null}
                              <span data-autocompact-where="">{row.where}</span>
                            </span>
                          </TableCell>
                          <TableCell>
                            <span className="font-mono text-xs text-ink tabular-nums">
                              {row.limit}
                            </span>
                          </TableCell>
                          <TableCell>
                            <span
                              data-autocompact-effective=""
                              className="block min-w-48 whitespace-normal text-ink-faint"
                            >
                              {row.effective}
                            </span>
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </TableFrame>
              )}
            </section>

            <section
              className="flex flex-col gap-2"
              aria-labelledby={`${ids}-history`}
            >
              <h3
                id={`${ids}-history`}
                className="text-sm font-semibold text-ink"
              >
                {HISTORY_HEADING}
              </h3>
              {log.error ? (
                <p role="alert" className="text-xs text-alert">
                  {log.error}
                </p>
              ) : log.rows === null ? (
                <p className="text-xs text-ink-faint">Reading…</p>
              ) : log.rows.length === 0 ? (
                <p className="text-xs text-ink-faint">{HISTORY_EMPTY}</p>
              ) : (
                <ul className="flex flex-col">
                  {log.rows.map((entry) => (
                    <li key={`${entry.session_id}:${entry.at_ms}`}>
                      <button
                        type="button"
                        data-autocompact-log={entry.session_id}
                        disabled={!ctx}
                        onClick={() => {
                          ctx?.openConversation(entry.session_id)
                          onOpenChange(false)
                        }}
                        className="flex min-h-9 w-full items-center gap-3 rounded-md px-2 text-left text-sm enabled:hover:bg-surface-hover focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50 pointer-coarse:min-h-11"
                      >
                        <span className="min-w-0 flex-1 truncate text-ink">
                          {entry.title}
                        </span>
                        <span className="shrink-0 font-mono text-xs text-ink-faint tabular-nums">
                          {historyChange(
                            entry.context_before,
                            entry.context_after,
                          )}
                        </span>
                        <span className="w-10 shrink-0 text-right font-mono text-meta text-ink-faint tabular-nums">
                          {entry.at_ms ? formatRelative(entry.at_ms) : ''}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </div>
        </PanelDialogContent>
      </Dialog>
      {editingProject ? (
        <ProjectDialog
          open
          onOpenChange={(next) => {
            if (!next) setEditingProject(null)
          }}
          project={editingProject}
          projects={chatProjects.projects}
          conversations={ctx?.conversations ?? []}
          initialSection="autocompact"
        />
      ) : null}
      {editingConversation && ctx ? (
        <ConversationDetailsDialog
          open
          conversation={editingConversation}
          onOpenChange={(next) => {
            if (!next) setEditingConversation(null)
          }}
          onSave={(draft) => ctx.saveDetails(editingConversation.id, draft)}
          projects={chatProjects.available ? chatProjects.projects : undefined}
          conversations={ctx.conversations}
          initialSection="autocompact"
        />
      ) : null}
    </>
  )
}

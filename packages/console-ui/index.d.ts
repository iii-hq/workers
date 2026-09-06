/**
 * @iii-dev/console-ui — the module surface injected worker UI imports.
 *
 * At runtime the console's import map resolves this specifier to
 * `/vendor/console-ui.js`, which re-exports the running SPA's engine
 * client, curated component library, and theme hook from
 * `window.__III_CONSOLE__` — one React tree, zero bytes bundled per
 * worker. This package is the compile-time face of that module: keep it
 * `external` in every worker UI build (the js entry throws if bundled).
 *
 * Component prop types are hand-modeled on the console's real components
 * (console/web/src/components/ui, lib/markdown, lib/syntax) and checked
 * against them by console/web/src/lib/console-ui-conformance.test.ts —
 * declared props must stay accepted by the real components. The console's
 * own components may accept more (Radix pass-through props); this file
 * declares the supported authoring surface.
 */

import type * as React from 'react'

/* ── the engine client ──────────────────────────────────────────────── */

/**
 * The tab's live engine client, narrowed for extensions: shared by the
 * whole tab, so `dispose()` is deliberately not carried.
 */
export interface ExtensionIii {
  /** `console-<uuid>`, per tab. */
  browserId: string
  trigger<T = unknown>(
    functionId: string,
    payload?: Record<string, unknown>,
    options?: { timeoutMs?: number },
  ): Promise<T>
  /**
   * Subscribe a browser-local handler. Handlers are namespaced per tab
   * (`<functionId>::<browserId>`) and default `metadata.internal = true`.
   */
  on<P = unknown>(functionId: string, handler: (payload: P) => void | Promise<void>): () => void
  registerTrigger(input: { type: string; function_id: string; config: Record<string, unknown> }): () => void
  addConnectionStateListener(handler: (state: unknown) => void): () => void
}

/* ── slot contracts ─────────────────────────────────────────────────── */

/**
 * Where the workspace pane hosting the page sits. `'right'` only for the
 * rightmost column of a multi-column tab — a single-column tab is `'left'`,
 * so pages can treat `'left'` as the default orientation.
 */
export type PanelSide = 'left' | 'right'

/** JSON context sent by an injected renderer to an injected page. */
export interface PanelContextEvent<T extends JsonValue = JsonValue> {
  /** Monotonic per-console-tab id; repeated context still produces an event. */
  id: number
  /** Registered page id that owns this context. */
  pageId: string
  context: T
}

export interface PanelOpenRequest<T extends JsonValue = JsonValue> {
  /** Registered extension page to place or reuse in the workspace. */
  pageId: string
  /** Worker-defined, JSON-serializable context delivered to that page. */
  context?: T
}

/** Props the host passes to every registered page render component. */
export interface PageRenderProps {
  panelSide: PanelSide
  /**
   * Stable id of the workspace tab whose pane hosts this render — the key
   * for per-tab UI state (workspace tabs persist across reloads). Empty
   * string when the page renders outside a workspace tab.
   */
  tabId: string
  /**
   * Close the pane hosting this page (a split drops the column; a
   * single-column tab detaches back to the attach affordance). Pass it to
   * `PageHeader`'s `onClose` — every page header carries the standard ✕.
   * Absent when the page renders outside a closable pane.
   */
  onRequestClose?: () => void
  /**
   * The active chat conversation's working directory, live: the page
   * re-renders with the new value when the user picks another folder in
   * chat, so filesystem-shaped pages can follow along. `null`/absent when
   * no conversation is active or none is set.
   */
  workingDir?: string | null
  /** Latest context sent through `host.panels.open()` for this page. */
  panelContext?: PanelContextEvent
  /** Active chat session id. Reactive pages use it to subscribe to exact
      Harness turn boundaries without receiving another chat's events. */
  conversationId?: string | null
  /**
   * Report unsaved work: `true` or a short label (e.g. the file name) while
   * the page holds something the user would lose, `false` once it is saved
   * or discarded. Closing the pane, its workspace, or the browser tab then
   * asks first. Absent on older consoles; feature-detect before calling.
   */
  setDirty?: (dirty: boolean | string) => void
  /**
   * Contribute commands while mounted: rows in the command palette, and keys
   * that fire only while focus is inside this pane. Register in an effect and
   * return its remover. Absent on older consoles; feature-detect.
   */
  commands?: PageCommandsApi
}

/** A binding in the console's shortcut syntax: `Mod+S`, `Shift+Enter`, or a
    sequence of chords separated by a space, `G L`. */
export type PageShortcut = string | { mac: readonly string[]; other: readonly string[] }

/** One thing a page can do for the keyboard; the palette lists it as
    `Page: Title` with its key on the right. */
export interface PageCommand {
  /** Unique within the page; the console namespaces it with the page id. */
  id: string
  title: string
  detail?: string
  keywords?: readonly string[]
  /** Honoured for commands registered by a mounted page only, while focus is
      inside its pane. A key the console already uses is refused with a
      warning; the row stays. */
  shortcut?: PageShortcut
  /** Fire while the caret is in a field. Default false. */
  firesWhileTyping?: boolean
  /** Checked when the palette opens and before the command runs; false hides
      the row and ignores the key. */
  enabled?: () => boolean
  run: () => void
}

export interface PageCommandsApi {
  register(commands: readonly PageCommand[]): () => void
}

/** One answer a palette source gives for a query. */
export interface PaletteSourceRow {
  id: string
  title: string
  detail?: string
  meta?: string
  keywords?: readonly string[]
  run: () => void
}

export interface PaletteSearchContext {
  /** The active chat session, for data that lives per conversation. */
  conversationId: string | null
  workingDir: string | null
  signal: AbortSignal
}

/** A live palette source: rows computed per query by a worker, registered
    from setup, alive only while the worker is connected. */
export interface PaletteSource {
  id: string
  title: string
  kind: 'file' | 'item'
  prefix?: string
  minQuery?: number
  search(query: string, context: PaletteSearchContext): Promise<readonly PaletteSourceRow[]>
}

export interface PaletteOpenOptions {
  query?: string
}

export interface PageRegistration {
  /** kebab-case, unique per tab; convention `<worker>-<name>`. Routes at `#/ext/<id>`. */
  id: string
  /** Nav label. */
  title: string
  /**
   * Configuration id whose standard action the Console injects into the page
   * header. Pages declare the relationship here instead of duplicating a
   * configuration button.
   */
  configurationId?: string
  /** The page body (right pane). Receives `PageRenderProps` — a plain
      `() => <Page />` render stays valid and simply ignores them. */
  render: React.ComponentType<PageRenderProps>
}

/**
 * A function-trigger message as renderers receive it (the fields a worker
 * renderer should rely on; the console may carry more).
 */
export interface FunctionTriggerMessage {
  id: string
  role: 'function-trigger'
  functionId: string
  /** Short user-facing action supplied by the agent_trigger wrapper. */
  description?: string
  input: unknown
  output?: unknown
  durationMs?: number
  running?: boolean
  pendingApproval?: boolean
  createdAt: number
}

/**
 * A function-trigger renderer. Injected renderers dispatch before the
 * first-party families; `null` always means "fall through".
 */
export interface FunctionTriggerRenderer {
  /** e.g. `state/page.js#renderer` */
  id: string
  /** The host calls `tryRender*` only for function ids claimed here. */
  isMatch(functionId: string): boolean
  tryRender(message: FunctionTriggerMessage): React.ReactNode | null
  tryRenderRunning?(message: FunctionTriggerMessage): React.ReactNode | null
  tryRenderPreview?(message: FunctionTriggerMessage): React.ReactNode | null
  /** Compact settled-state surface shown directly in the chat feed. */
  tryRenderDisplay?(message: FunctionTriggerMessage): React.ReactNode | null
  FunctionIdLabel?: React.ComponentType<{ functionId: string }>
  primaryTabLabel?: string
  /**
   * Presentation hints owned by the worker. `display` makes a successful
   * non-null render visible in chat without opening the raw call details.
   */
  metadata?: {
    display?: boolean
    /** Keep the display as the card header and expand details underneath it. */
    displayAction?: 'expand'
  }
  /**
   * Redact the raw request/response before the console DISPLAYS OR COPIES
   * it. The card's `raw json` tab renders `message.input` / `message.output`
   * verbatim and its copy button puts the same value on the clipboard, so a
   * card that hides a secret in its own rendering has not contained it —
   * declare the secret here and the host applies it at both exits.
   *
   * Consulted only for messages this renderer's `isMatch` claims (the first
   * claiming renderer that declares it wins), once for the request and once
   * for the response.
   *
   * Receives an arbitrary JSON-ish value (object, array, string, number,
   * boolean, `null`, or `undefined` — whatever rode on the wire) and returns
   * the redacted copy. Must be PURE and TOTAL: never mutate the input, never
   * do I/O, never throw for any shape (cycles included). It runs inside the
   * host card's render; a throw is fenced and fails CLOSED — the pane shows
   * a "redaction failed" placeholder rather than the raw value, so a bug
   * here costs you the view, never the secret.
   */
  redactRaw?(value: unknown): unknown
}

/**
 * A host-normalized trigger activity. The host owns the surrounding message
 * chrome, raw details, and lifecycle controls; trigger renderers receive this
 * common shape only to render the source-specific section.
 */
export interface TriggerActivityMessage {
  /** Stable activity/message id. */
  id: string
  kind: 'registration' | 'fired' | 'retirement'
  /** e.g. `cron`, `state`, or a worker-defined trigger source. */
  triggerType: string
  config?: unknown
  label?: string
  /** What this event means in user-facing prose (`metadata.action`). */
  action?: string
  conditions?: readonly unknown[]
  delivery: { kind: 'notify' } | { kind: 'call'; functionId: string }
  lifecycle: {
    /** Whether the binding remains usable after this activity. */
    state: 'active' | 'retired'
    once: boolean
    maxFires?: number
    expiresAt?: number
    fires: number
  }
  subscriptionId?: string
  triggerId?: string
  payload?: unknown
  firedAt?: number
  note?: string
  outcome?: 'delivered' | 'delivery_failed' | 'skipped' | 'expired' | 'unregistered' | 'invalidated'
  retirementReason?: 'once_consumed' | 'max_fires' | 'expired' | 'unregistered' | 'invalidated' | 'exhausted'
}

/** Worker-owned presentation hooks for one trigger type. The base `tryRender`
 * remains the source-section hook for compatibility. Optional detail and
 * display hooks replace the expanded Terminal surface and compact timeline
 * content while the host keeps disclosure and raw-data access. */
export interface TriggerActivityRenderer {
  /** e.g. `cron/page.js#trigger-activity` */
  id: string
  isMatch(triggerType: string): boolean
  /** Source-specific content used by the host's generic detail view. */
  tryRender(activity: TriggerActivityMessage): React.ReactNode | null
  /** Complete expanded Terminal tab. `null` keeps the generic detail view. */
  tryRenderDetails?(activity: TriggerActivityMessage): React.ReactNode | null
  /** Compact clickable timeline content. Do not return interactive children. */
  tryRenderDisplay?(activity: TriggerActivityMessage): React.ReactNode | null
  /** Redact registration/fire raw values before display or copy. */
  redactRaw?(value: unknown): unknown
}

export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue }

/**
 * Props for a configuration form (the `configForms` slot). Every configurable
 * worker supplies its own interface; JSON Schema validates drafts but never
 * generates UI. Dirty tracking, save/reset, and error mapping stay host-owned.
 */
export interface ConfigFormProps {
  /** Configuration id, e.g. `state`. */
  id: string
  /** `null` = the worker registered a value but no JSON schema. */
  schema: Record<string, unknown> | null
  value: JsonValue
  onChange(next: JsonValue): void
  /** JSON-pointer → message, merged client + server validation. */
  errors?: ReadonlyMap<string, string>
  /** Deep-link focus request; honoring it is the override's job. */
  focusField?: readonly string[]
}

/** Layout the Console should reserve for a configuration-form override. */
export type ConfigFormLayout = 'contained' | 'full'

export interface ConfigFormRegistrationOptions {
  /**
   * `contained` keeps the standard centered form column and host scroll.
   * `full` gives the override all available width and height; the override
   * must then manage any scrolling inside its own layout.
   */
  layout?: ConfigFormLayout
}

/** Provider-owned editor rendered inside the chat model picker. */
export interface ProviderConfigFormProps {
  providerId: string
  schema: Record<string, unknown> | null
  value: JsonValue
  onChange(next: JsonValue): void
  errors?: ReadonlyMap<string, string>
  configured?: boolean
  available?: boolean
  modelCount: number
}

/**
 * Props a session chip receives from the chat host. Chips fetch their own
 * data through `host.iii`; the host only identifies the session and what
 * it already knows about the resolved model.
 */
export interface SessionChipProps {
  sessionId: string
  /** Resolved model id for the session, when known. */
  modelId?: string
  /** Context window (tokens) of the resolved model, from the catalog. */
  contextWindow?: number
}

/**
 * A per-session status chip rendered in the chat header's right cluster
 * (the `chat` slot). Duplicate `id`: last registration wins — and some
 * ids also supersede a built-in affordance (`context` replaces the
 * host's estimate-based context meter).
 */
export interface SessionChipRegistration {
  /** kebab-case, e.g. `context`; convention `<worker>-<name>` otherwise. */
  id: string
  render: React.ComponentType<SessionChipProps>
}

/** Props a chat footer turn-summary receives. */
export interface SessionTurnSummaryProps {
  sessionId: string
  /** Whether this session is currently producing a turn. */
  isStreaming: boolean
}

/**
 * A compact per-session summary rendered immediately above the composer.
 * Duplicate `id`: last registration wins.
 */
export interface SessionTurnSummaryRegistration {
  /** kebab-case; convention `<worker>-<name>`. */
  id: string
  render: React.ComponentType<SessionTurnSummaryProps>
}

/** Props a composer toolbar action receives. */
export interface ComposerActionProps {
  /** Active session id, or `null` while the conversation has none yet. */
  sessionId: string | null
  /** Whether this session is currently producing a turn. */
  isStreaming: boolean
}

/**
 * An icon-sized action rendered in the composer's toolbar, beside the
 * attach button. Duplicate `id`: last registration wins.
 */
export interface ComposerActionRegistration {
  /** kebab-case; convention `<worker>-<name>`. */
  id: string
  render: React.ComponentType<ComposerActionProps>
}

/** Props for a worker-owned annotation detail rendered in the transcript. */
export interface TranscriptAnnotationProps {
  version: number
  summary: string
  data: JsonValue
}

/** A transcript annotation renderer, selected by its exact annotation id. */
export interface TranscriptRendererRegistration {
  id: string
  render: React.ComponentType<TranscriptAnnotationProps>
}

/**
 * What `setup(host)` receives. Every registrar returns an unregister fn AND
 * is auto-tracked: the loader runs all of them on dispose.
 */
export interface Host {
  iii: ExtensionIii
  /** The curated component record — same components as the named exports below. */
  components: Record<string, React.ComponentType<any>>
  useTheme(): 'light' | 'dark'
  /** Stable namespaced CSS recipes for lists, cards, panels and controls. */
  uiClasses: UiClasses
  /** The script's asset path, e.g. `state/page.js`. */
  path: string
  pages: { register(page: PageRegistration): () => void }
  /**
   * The console's remembered working directories, most recent first — the
   * same list its chat composer offers. A page with its own root picker
   * lists them so both pickers agree on where work happens.
   */
  workspace?: { recentDirectories(): string[] }
  /**
   * Palette rows for a page this script owns, alive exactly as long as the
   * script. For a page that may not be open yet; `run` usually calls
   * `panels.open`. Keys are honoured only through `PageRenderProps.commands`.
   * Absent on older consoles; feature-detect.
   */
  commands?: {
    register(pageId: string, commands: readonly PageCommand[]): () => void
  }
  /** Live palette sources and a way to open the palette on a query. Absent
      on older consoles; feature-detect. */
  palette?: {
    registerSource(source: PaletteSource): () => void
    open(options?: PaletteOpenOptions): void
  }
  functionTriggers: {
    register(renderer: FunctionTriggerRenderer): () => void
  }
  /**
   * Optional on consoles that predate source-specific trigger activity UI.
   * Feature-detect before registering when a worker supports older consoles.
   */
  triggerRenderers?: {
    register(renderer: TriggerActivityRenderer): () => void
  }
  /**
   * Optional on older consoles. Feature-detect before opening contextual
   * pages when a worker must remain compatible with them.
   */
  panels?: {
    /** Place/reuse a registered page and deliver its worker-defined context. */
    open(request: PanelOpenRequest): void
  }
  configForms: {
    register(
      configurationId: string,
      component: React.ComponentType<ConfigFormProps>,
      options?: ConfigFormRegistrationOptions,
    ): () => void
  }
  /** Optional on consoles that predate provider-specific configuration UI. */
  providerConfigForms?: {
    register(providerId: string, component: React.ComponentType<ProviderConfigFormProps>): () => void
  }
  /**
   * Optional: absent on consoles that predate session chips. Feature-detect
   * with `host.chat?.registerSessionChip` — newer slot namespaces are always
   * declared optional so worker scripts degrade without casts.
   */
  chat?: {
    /**
     * Hand text and files to the active conversation's composer, the way a
     * drop or a paste would, and put the caret there. Files become
     * attachments. Absent on older consoles; feature-detect.
     */
    compose?(draft: { text?: string; files?: File[] }): void
    registerSessionChip(chip: SessionChipRegistration): () => void
    /** Optional on consoles that predate the footer turn-summary slot. */
    registerTurnSummary?(summary: SessionTurnSummaryRegistration): () => void
    /** Optional on consoles that predate the composer toolbar slot. */
    registerComposerAction?(action: ComposerActionRegistration): () => void
    registerTranscriptRenderer?(renderer: TranscriptRendererRegistration): () => void
    /** Optional on consoles that predate worker-driven conversation switching. */
    selectConversation?(sessionId: string): void
    /**
     * Ask the mounted conversation to adopt a validated working directory.
     * Call this only from an explicit user action such as "Use for chat".
     */
    requestWorkingDirectoryChange?(request: { sessionId: string; path: string }): boolean
    /** Live composer model for a conversation, including unsaved drafts. */
    composerModel?(conversationId?: string | null): string | null
  }
}

/** The ONLY required export of a script asset. */
export type SetupFn = (host: Host) => void | (() => void) | Promise<void | (() => void)>

/* ── module-level api (same objects the Host carries) ───────────────── */

export declare const iii: ExtensionIii
export declare const components: Record<string, React.ComponentType<any>>
export declare function useTheme(): 'light' | 'dark'
/** Design-token names, for documentation/tooling; styling just uses `var(--color-*)`. */
export declare const tokens: readonly string[]
/** Stable namespaced CSS recipes; state is expressed through `data-*` attributes. */
export declare const uiClasses: UiClasses

export interface UiClasses {
  readonly list: 'iii-ui-list'
  readonly listGroup: 'iii-ui-list-group'
  readonly listGroupLabel: 'iii-ui-list-group__label'
  readonly listItem: 'iii-ui-list-item'
  readonly listItemIcon: 'iii-ui-list-item__icon'
  readonly listItemContent: 'iii-ui-list-item__content'
  readonly listItemTitle: 'iii-ui-list-item__title'
  readonly listItemDescription: 'iii-ui-list-item__description'
  readonly listItemMeta: 'iii-ui-list-item__meta'
  readonly card: 'iii-ui-card'
  readonly cardHeader: 'iii-ui-card__header'
  readonly cardBody: 'iii-ui-card__body'
  readonly cardHighlight: 'iii-ui-card-highlight'
  readonly collapsibleCard: 'iii-ui-collapsible-card'
  readonly collapsibleCardTrigger: 'iii-ui-collapsible-card__trigger'
  readonly collapsibleCardContent: 'iii-ui-collapsible-card__content'
  readonly collapsibleCardContentInner: 'iii-ui-collapsible-card__content-inner'
  readonly panel: 'iii-ui-panel'
  readonly panelHeader: 'iii-ui-panel__header'
  readonly panelBody: 'iii-ui-panel__body'
  readonly chip: 'iii-ui-chip'
  readonly icon: 'iii-ui-icon'
  readonly tableViewport: 'iii-ui-table-viewport'
  readonly tableFrame: 'iii-ui-table-frame'
  readonly table: 'iii-ui-table'
  readonly tableHeader: 'iii-ui-table__header'
  readonly tableBody: 'iii-ui-table__body'
  readonly tableFooter: 'iii-ui-table__footer'
  readonly tableRow: 'iii-ui-table__row'
  readonly tableHead: 'iii-ui-table__head'
  readonly tableCell: 'iii-ui-table__cell'
  readonly tableCaption: 'iii-ui-table__caption'
  readonly tabsList: 'iii-ui-tabs-list'
  readonly tab: 'iii-ui-tab'
  readonly tabIcon: 'iii-ui-tab__icon'
  readonly segmentedControl: 'iii-ui-segmented'
  readonly segmentedItem: 'iii-ui-segmented__item'
  readonly field: 'iii-ui-field'
  readonly fieldLabel: 'iii-ui-field__label'
  readonly fieldDescription: 'iii-ui-field__description'
  readonly fieldError: 'iii-ui-field__error'
  readonly switch: 'iii-ui-switch'
  readonly switchInput: 'iii-ui-switch__input'
  readonly switchThumb: 'iii-ui-switch__thumb'
  readonly settingsSection: 'iii-ui-settings-section'
  readonly settingsSectionHeader: 'iii-ui-settings-section__header'
  readonly settingsSectionCopy: 'iii-ui-settings-section__copy'
  readonly settingsSectionTitle: 'iii-ui-settings-section__title'
  readonly settingsSectionDescription: 'iii-ui-settings-section__description'
  readonly settingsSectionAction: 'iii-ui-settings-section__action'
  readonly settingsList: 'iii-ui-settings-list'
  readonly settingsRow: 'iii-ui-settings-row'
  readonly settingsRowInner: 'iii-ui-settings-row__inner'
  readonly settingsRowCopy: 'iii-ui-settings-row__copy'
  readonly settingsRowLabel: 'iii-ui-settings-row__label'
  readonly settingsRowDescription: 'iii-ui-settings-row__description'
  readonly settingsRowMeta: 'iii-ui-settings-row__meta'
  readonly settingsRowControl: 'iii-ui-settings-row__control'
  readonly motionControl: 'iii-ui-motion-control'
  readonly motionPanel: 'iii-ui-motion-panel'
  readonly motionOverlay: 'iii-ui-motion-overlay'
}

/* ── the shared component library ───────────────────────────────────── */

/** A numbered pin with a note, positioned as fractions of the picture. */
export type AnnotationKind = 'pin' | 'rect' | 'arrow'
export type AnnotationTool = AnnotationKind | 'select'
export interface Annotation {
  id: string
  x: number
  y: number
  note: string
  kind?: AnnotationKind
  x2?: number
  y2?: number
  color?: string
  /** What the pin points at, when the page knows: an element, a window, a page. */
  label?: string
}

export interface AnnotationLayerProps {
  annotations: readonly Annotation[]
  /** The picture's pixel size, for the `object-fit: contain` box. */
  image: { width: number; height: number }
  /** Clicks add pins while active. */
  active: boolean
  selectedId?: string | null
  onAdd?: (x: number, y: number) => void
  onSelect?: (id: string | null) => void
  onMove?: (id: string, x: number, y: number) => void
  onRemove?: (id: string) => void
  /** With it, the selected pin opens a callout that edits its note in place. */
  onNote?: (id: string, note: string) => void
  /** The active tool: pin (default) drops pins; rect / arrow draw on drag. */
  tool?: AnnotationTool
  /** Colour for a newly drawn shape. */
  drawColor?: string
  onAddShape?: (kind: 'rect' | 'arrow', x: number, y: number) => void
  onResizeShape?: (x2: number, y2: number) => void
  onEndShape?: () => void
  className?: string
  /** The picture element, rendered by the caller. */
  children: React.ReactNode
}
/** Numbered pins over a picture: focusable, Delete removes, arrows nudge. */
export declare const AnnotationLayer: React.ComponentType<AnnotationLayerProps>

export interface AnnotationListProps {
  annotations: readonly Annotation[]
  selectedId?: string | null
  onSelect?: (id: string | null) => void
  onNote: (id: string, note: string) => void
  onRemove: (id: string) => void
  emptyText?: string
  className?: string
}
/** One note field per pin; Enter moves to the next, arrows walk the rows. */
export declare const AnnotationList: React.ComponentType<AnnotationListProps>

export interface AnsiTextProps {
  /** Raw terminal output, ANSI escapes included. */
  text: string
  className?: string
}
/** Terminal text with its ANSI SGR colors mapped onto the design tokens
    (red→alert, green→ok, yellow→warn, blue/cyan/magenta→accent,
    bold→semibold). Extended-color params are consumed, every other
    CSI/OSC sequence is stripped, and pathological input falls back to
    stripped plain text. The console's one ANSI renderer — like
    `CodeEditor`, never bundle an ANSI parser into a worker asset; import
    this instead. Inherits the parent's font, so put it inside a
    whitespace-preserving mono pane (`TerminalStream` does). */
export declare const AnsiText: React.ComponentType<AnsiTextProps>

export type BadgeVariant = 'default' | 'ok' | 'warn' | 'alert' | 'accent'
export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  /** Semantic tone. `ok` is positive, `default` is neutral. */
  variant?: BadgeVariant
}
/** Compact semantic status label shared by the Console and worker UIs. */
export declare const Badge: React.ComponentType<BadgeProps>

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'primary' | 'ghost' | 'pill' | 'icon' | 'terminal' | 'wiggle'
  size?: 'sm' | 'md' | 'lg' | 'icon'
  /** Render as the child element (Radix Slot) instead of a `<button>`. */
  asChild?: boolean
}
export declare const Button: React.ComponentType<ButtonProps & React.RefAttributes<HTMLButtonElement>>

export interface CardProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Neutral selected treatment: surface, edge and ink; never accent. */
  selected?: boolean
  interactive?: boolean
}
export declare const Card: React.ComponentType<CardProps & React.RefAttributes<HTMLDivElement>>
export declare const CardHeader: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>
export declare const CardBody: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>
/** Borderless neutral inset for related content that needs emphasis inside a card. */
export declare const CardHighlight: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>

export interface CollapsibleCardProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Controlled expanded state. */
  open?: boolean
  /** Initial expanded state when the component is uncontrolled. */
  defaultOpen?: boolean
  /** Called after the trigger requests an expanded-state change. */
  onOpenChange?(open: boolean): void
  /** Prevent the trigger from changing the expanded state. */
  disabled?: boolean
}
/**
 * Accessible shared card disclosure with an auto-height transition. Content
 * stays mounted while collapsed, and global motion tokens honor reduced motion.
 */
export declare const CollapsibleCard: React.ComponentType<CollapsibleCardProps & React.RefAttributes<HTMLDivElement>>
export type CollapsibleCardTriggerProps = React.ButtonHTMLAttributes<HTMLButtonElement>
export declare const CollapsibleCardTrigger: React.ComponentType<
  CollapsibleCardTriggerProps & React.RefAttributes<HTMLButtonElement>
>
export type CollapsibleCardContentProps = React.HTMLAttributes<HTMLElement>
export declare const CollapsibleCardContent: React.ComponentType<
  CollapsibleCardContentProps & React.RefAttributes<HTMLElement>
>

export type ChipTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger'
export interface ChipProps extends React.HTMLAttributes<HTMLSpanElement> {
  tone?: ChipTone
  selected?: boolean
}
export declare const Chip: React.ComponentType<ChipProps & React.RefAttributes<HTMLSpanElement>>

export type TableDensity = 'comfortable' | 'compact'
export interface TableProps extends React.TableHTMLAttributes<HTMLTableElement> {
  density?: TableDensity
}
export declare const TableViewport: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>
export declare const TableFrame: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>
export declare const Table: React.ComponentType<TableProps & React.RefAttributes<HTMLTableElement>>
export declare const TableHeader: React.ComponentType<
  React.HTMLAttributes<HTMLTableSectionElement> & React.RefAttributes<HTMLTableSectionElement>
>
export declare const TableBody: React.ComponentType<
  React.HTMLAttributes<HTMLTableSectionElement> & React.RefAttributes<HTMLTableSectionElement>
>
export declare const TableFooter: React.ComponentType<
  React.HTMLAttributes<HTMLTableSectionElement> & React.RefAttributes<HTMLTableSectionElement>
>
export interface TableRowProps extends React.HTMLAttributes<HTMLTableRowElement> {
  interactive?: boolean
  selected?: boolean
}
export declare const TableRow: React.ComponentType<TableRowProps & React.RefAttributes<HTMLTableRowElement>>
export declare const TableHead: React.ComponentType<
  React.ThHTMLAttributes<HTMLTableCellElement> & React.RefAttributes<HTMLTableCellElement>
>
export declare const TableCell: React.ComponentType<
  React.TdHTMLAttributes<HTMLTableCellElement> & React.RefAttributes<HTMLTableCellElement>
>
export declare const TableCaption: React.ComponentType<
  React.HTMLAttributes<HTMLTableCaptionElement> & React.RefAttributes<HTMLTableCaptionElement>
>

export interface ImageViewerProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Image source; `null`/`undefined` renders the unavailable state. */
  src?: string | null
  /** Accessible description of the picture. */
  alt: string
  /** Caption: the attachment name or the file's relative path, never a host path. */
  title?: string
  /** Secondary caption, e.g. a byte size or MIME type. */
  description?: string
  /** Why the source is unavailable, when it is. */
  unavailableReason?: string
  /** Extra controls rendered in the toolbar before Close. */
  actions?: React.ReactNode
}
/**
 * Full-screen image viewer: wheel/pinch zoom about the pointer, drag pans,
 * double-click toggles fit and actual size, `+`/`-`/`0`/`1` and arrows on the
 * keyboard, Escape closes and focus returns to the opener. Handles loading,
 * decode failure, missing source and oversized data URLs without freezing.
 */
export declare const ImageViewer: React.ComponentType<ImageViewerProps>
export interface ImageThumbnailButtonProps extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'title'> {
  /** What opens: the viewer's caption. */
  title: string
}
/** The opener around a thumbnail: zoom cursor, accessible "View <title>" name. */
export declare const ImageThumbnailButton: React.ComponentType<
  ImageThumbnailButtonProps & React.RefAttributes<HTMLButtonElement>
>

export interface IconButtonProps extends Omit<ButtonProps, 'size'> {
  /** Required accessible name; also the default tooltip content. */
  label: string
  tooltip?: React.ReactNode | false
  tooltipSide?: 'top' | 'right' | 'bottom' | 'left'
}
export declare const IconButton: React.ComponentType<IconButtonProps & React.RefAttributes<HTMLButtonElement>>

export interface ConfirmDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: React.ReactNode
  /** Lines listed under the description, e.g. the unsaved items at stake. */
  details?: readonly string[]
  confirmLabel?: string
  cancelLabel?: string
  onConfirm: () => void
  onCancel?: () => void
}
/**
 * The console's confirmation in place of `window.confirm`: cancel owns
 * initial focus, Escape and the close control cancel.
 */
export declare const ConfirmDialog: React.ComponentType<ConfirmDialogProps>

/** Root is state-only; compose with `DialogTrigger`/`DialogContent`. */
export interface DialogProps {
  open?: boolean
  defaultOpen?: boolean
  onOpenChange?(open: boolean): void
  modal?: boolean
  children?: React.ReactNode
}
export declare const Dialog: React.ComponentType<DialogProps>
export interface DialogTriggerProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  asChild?: boolean
}
export declare const DialogTrigger: React.ComponentType<DialogTriggerProps>
export declare const DialogClose: React.ComponentType<DialogTriggerProps>
/** Portalled, centered, with overlay and close affordance built in. */
export declare const DialogContent: React.ComponentType<React.HTMLAttributes<HTMLDivElement>>
export declare const DialogTitle: React.ComponentType<React.HTMLAttributes<HTMLHeadingElement>>
export declare const DialogDescription: React.ComponentType<React.HTMLAttributes<HTMLParagraphElement>>

export interface DropdownMenuProps {
  open?: boolean
  defaultOpen?: boolean
  onOpenChange?(open: boolean): void
  modal?: boolean
  children?: React.ReactNode
}
export declare const DropdownMenu: React.ComponentType<DropdownMenuProps>
export interface DropdownMenuTriggerProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  asChild?: boolean
}
export declare const DropdownMenuTrigger: React.ComponentType<DropdownMenuTriggerProps>
export interface DropdownMenuContentProps extends React.HTMLAttributes<HTMLDivElement> {
  side?: 'top' | 'right' | 'bottom' | 'left'
  align?: 'start' | 'center' | 'end'
  sideOffset?: number
}
export declare const DropdownMenuContent: React.ComponentType<DropdownMenuContentProps>
export interface DropdownMenuItemProps extends React.HTMLAttributes<HTMLDivElement> {
  disabled?: boolean
  onSelect?(event: Event): void
}
export declare const DropdownMenuItem: React.ComponentType<DropdownMenuItemProps>
export declare const DropdownMenuLabel: React.ComponentType<React.HTMLAttributes<HTMLDivElement>>
export declare const DropdownMenuSeparator: React.ComponentType<React.HTMLAttributes<HTMLDivElement>>

export interface EmptyStateProps {
  icon?: React.ComponentType<{ className?: string }>
  title: string
  description: string
  action?: { label: string; onClick: () => void }
}
export declare const EmptyState: React.ComponentType<EmptyStateProps>

export interface ErrorBoundaryProps {
  children: React.ReactNode
  fallback?: (error: Error) => React.ReactNode
}
export declare const ErrorBoundary: React.ComponentType<ErrorBoundaryProps>

/** One side of a `FileDiff` — a whole file's text, not a patch. */
export interface FileDiffSide {
  /** Display name; also infers the syntax-highlight language. */
  name: string
  contents: string
}
export type FileDiffEditState = 'loading' | 'ready' | 'error'
export interface FileDiffProps {
  /** Pass empty `contents` for a created (old) / deleted (new) file. */
  oldFile: FileDiffSide
  newFile: FileDiffSide
  diffStyle?: 'unified' | 'split'
  /** Long lines wrap by default; `'scroll'` preserves strict columns. */
  overflow?: 'scroll' | 'wrap'
  /** Intraline emphasis. `'none'` keeps only whole-line highlighting. */
  lineDiffType?: 'word-alt' | 'word' | 'char' | 'none'
  /** Ignore leading/trailing whitespace when computing changed lines. */
  ignoreWhitespace?: boolean
  /** Render the file body folded; useful for collapse-all review controls. */
  collapsed?: boolean
  /** Expand every unchanged line instead of the compact hunk view. */
  expandUnchanged?: boolean
  /** Hide the built-in file header when the caller supplies its own. */
  disableFileHeader?: boolean
  /** Enable direct editing of the new-file side. The editor loads lazily. */
  edit?: boolean
  /** Receives the complete current new-file body after each edit. */
  onChange?(contents: string): void
  /** Reports whether the lazily loaded inline editor can accept input. */
  onEditStateChange?(state: FileDiffEditState): void
  className?: string
}
/** The console's one file-diff surface — the diff is computed from the two
    full file bodies and rendered by the console's bundled diff engine,
    following the active theme. Like `CodeEditor`, never bundle a diff
    renderer into a worker asset; import this instead. */
export declare const FileDiff: React.ComponentType<FileDiffProps>

/** Controlled string input (`onChange` receives the value, not the event). */
export interface InputProps extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'onChange' | 'value'> {
  value: string
  onChange: (next: string) => void
  /** @deprecated Inputs preserve case by default; retained for compatibility. */
  preserveCase?: boolean
}
export declare const Input: React.ComponentType<InputProps & React.RefAttributes<HTMLInputElement>>

export interface RawValueInputProps extends Omit<InputProps, 'className'> {
  /** Human label used by the explicit replacement action. */
  label: string
  kind: 'environment' | 'custom'
  replacementLabel: React.ReactNode
  onUseLiteral: () => void
  className?: string
  inputClassName?: string
}
/** Opaque/template editor that only converts to a typed literal explicitly. */
export declare const RawValueInput: React.ComponentType<
  RawValueInputProps & React.RefAttributes<HTMLInputElement>
>

export interface SwitchProps
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'children' | 'className' | 'type'> {
  /** Classes apply to the visual control; native input props stay on the checkbox. */
  className?: string
}
/** Native checkbox semantics with shared switch presentation and touch target. */
export declare const Switch: React.ComponentType<SwitchProps & React.RefAttributes<HTMLInputElement>>

export declare const List: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>
export declare const ListGroup: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>
export declare const ListGroupLabel: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>
export interface ListItemProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  selected?: boolean
  leading?: React.ReactNode
  label?: React.ReactNode
  description?: React.ReactNode
  trailing?: React.ReactNode
}
export declare const ListItem: React.ComponentType<ListItemProps & React.RefAttributes<HTMLButtonElement>>

/* ── page chrome: THE layout design system for injected pages ─────────
   Every worker page composes the same five pieces so panes stay visually
   identical across workers (and console-native screens):

     <PageShell>
       <PageHeader icon? title description? actions? onClose={onRequestClose} />
       <PageBody side={panelSide}>
         <PageSidebar>…navigation…</PageSidebar>
         <PageMain>…workspace…</PageMain>
       </PageBody>
     </PageShell>

   PageHeader renders the standard ✕ when `onClose` is present — wire it
   to `PageRenderProps.onRequestClose`. */

export interface PageShellProps extends React.ComponentProps<'div'> {}
/** The pane's root column — fills the pane, `--color-panel` background. */
export declare const PageShell: React.ComponentType<PageShellProps>

export interface PageHeaderProps {
  /** Identity glyph, rendered at 16px in faint ink (any svg fits). */
  icon?: React.ReactNode
  /** The page's human-readable name. Technical identifiers belong in `description`. */
  title?: React.ReactNode
  /** One short descriptor; truncates before anything else gives. */
  description?: React.ReactNode
  /** Free-form middle content (fills the flexible gap before actions). */
  children?: React.ReactNode
  /** Right-side controls, rendered before the close affordance. */
  actions?: React.ReactNode
  /** Close the hosting pane — renders the standard ✕ when present.
      Wire to `PageRenderProps.onRequestClose`. */
  onClose?: () => void
  className?: string
}
/** The standard pane top bar: slightly raised, hairline bottom edge. */
export declare const PageHeader: React.ComponentType<PageHeaderProps>

export interface PageBodyProps extends React.HTMLAttributes<HTMLDivElement> {
  /** The pane's side of the tab (`PageRenderProps.panelSide`) — `right`
      mirrors the row so the sidebar hugs the pane's outer edge. */
  side?: 'left' | 'right'
}
/** The row under the header; separates children with a hairline gap. */
export declare const PageBody: React.ComponentType<PageBodyProps>

export interface PageSidebarProps extends React.HTMLAttributes<HTMLElement> {
  /** Accessible name used by the aside, toggle, tooltip and resize handle. */
  label?: string
  /** Which outer edge the sidebar hugs. */
  side?: 'left' | 'right'
  /** Optional standard top row rendered beside the stable collapse toggle. */
  header?: React.ReactNode
  /** Compact actions rendered below the toggle in the collapsed rail. */
  collapsedActions?: React.ReactNode
  /** Controlled expanded width. */
  width?: number
  /** Initial expanded width for host-owned state. */
  defaultWidth?: number
  minWidth?: number
  maxWidth?: number
  onWidthChange?: (width: number) => void
  collapsible?: boolean
  collapsed?: boolean
  defaultCollapsed?: boolean
  onCollapsedChange?: (collapsed: boolean) => void
  resizable?: boolean
  /** Host-owned persistence/synchronization key; no storage logic ships in worker bundles. */
  storageKey?: string
  /** Activates the responsive presentation when the page already knows its container is narrow; never changes the saved wide preference. */
  narrow?: boolean
  /** Host-owned PageBody breakpoint for the responsive presentation. */
  narrowBelow?: number
  /** `inline` (default) makes narrow navigation full-width for drill-in flows; `drawer` keeps a rail beside the main column and opens navigation as an overlay. */
  narrowMode?: 'inline' | 'drawer'
}
/** Shared host-owned navigation column; fixed by default, optionally collapsible/resizable. */
export declare const PageSidebar: React.ComponentType<PageSidebarProps>

/** The primary workspace column: `--color-panel`, takes what's left. */
export declare const PageMain: React.ComponentType<React.HTMLAttributes<HTMLElement>>

export declare const Panel: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>
export declare const PanelHeader: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>
export declare const PanelBody: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>

export interface SelectOption<T extends string = string> {
  value: T
  label: string
  /** Optional hover tooltip on the option row. */
  title?: string
  /** Supporting copy shown below the label in menus and mobile sheets. */
  description?: string
  disabled?: boolean
}
export interface SelectGroup<T extends string = string> {
  label: string
  options: SelectOption<T>[]
}
export interface SelectProps<T extends string = string> {
  /** `undefined` (or a value matching no option) renders the `placeholder`. */
  value: T | undefined
  options?: SelectOption<T>[]
  groups?: SelectGroup<T>[]
  onChange: (next: T) => void
  disabled?: boolean
  className?: string
  /** ID applied to the visible trigger so a `<label htmlFor>` can target it. */
  id?: string
  /** Form name. The controlled value is mirrored through a hidden input. */
  name?: string
  /** Stable configuration path applied to the visible trigger for deep-link focus. */
  'data-field'?: string
  appearance?: 'default' | 'inline'
  showChevron?: boolean
  'aria-label'?: string
  'aria-busy'?: boolean
  'aria-invalid'?: React.AriaAttributes['aria-invalid']
  'aria-describedby'?: string
  placeholder?: string
  sheetTitle?: React.ReactNode
  sheetDescription?: React.ReactNode
  /** Render a leading option that clears the selection (calls `onClear`, not `onChange`). */
  allowEmpty?: boolean
  emptyLabel?: string
  onClear?: () => void
  renderGroupHeader?: (group: SelectGroup<T>) => React.ReactNode
}
export declare const Select: <T extends string = string>(props: SelectProps<T>) => React.ReactNode

export interface SegmentedControlOption<T extends string = string> {
  value: T
  label: React.ReactNode
  title?: string
  /** Defaults to a semantic 16px icon inferred from `value`; `false` hides it. */
  icon?: React.ReactNode | false
}
export interface SegmentedControlProps<T extends string = string> {
  value: T
  onChange: (next: T) => void
  options: SegmentedControlOption<T>[]
  className?: string
  itemClassName?: string
  activeItemClassName?: string
  variant?: 'tabs' | 'radio'
  /** Show only the semantic icon and expose each label through Tooltip. */
  iconOnly?: boolean
  'aria-label'?: string
}
export declare const SegmentedControl: <T extends string = string>(props: SegmentedControlProps<T>) => React.ReactNode

export interface SelectorOption<T extends string = string> {
  value: T
  label: string
  description?: string
  keywords?: readonly string[]
  disabled?: boolean
}
export interface SelectorGroup<T extends string = string> {
  label: string
  options: readonly SelectorOption<T>[]
}
export interface SelectorProps<T extends string = string> {
  value: T | undefined
  options?: readonly SelectorOption<T>[]
  groups?: readonly SelectorGroup<T>[]
  onChange: (next: T) => void
  open?: boolean
  onOpenChange?: (open: boolean) => void
  query?: string
  onQueryChange?: (query: string) => void
  shouldFilter?: boolean
  loading?: boolean
  error?: React.ReactNode
  validationMessage?: React.ReactNode
  disabled?: boolean
  invalid?: boolean
  className?: string
  contentClassName?: string
  /** ID applied to the visible trigger so a `<label htmlFor>` can target it. */
  id?: string
  /** Form name. The selected value is mirrored through a hidden input. */
  name?: string
  /** Stable configuration path applied to the visible trigger for deep-link focus. */
  'data-field'?: string
  placeholder?: string
  searchPlaceholder?: string
  emptyMessage?: React.ReactNode
  loadingMessage?: React.ReactNode
  allowEmpty?: boolean
  emptyLabel?: string
  onClear?: () => void
  /** Optional free-form commit for selectors such as arbitrary attribute keys. */
  onCreate?: (query: string) => void
  createOptionLabel?: (query: string) => React.ReactNode
  triggerIcon?: React.ReactNode
  'aria-label': string
  'aria-invalid'?: React.AriaAttributes['aria-invalid']
  'aria-describedby'?: string
}
export declare const Selector: <T extends string = string>(props: SelectorProps<T>) => React.ReactNode

export interface SettingsDeckProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, 'children' | 'title'> {
  /** Whether the detail level is visible. The overview is shown when false. */
  open: boolean
  /** Collection, summary, or empty state shown at the deck's root level. */
  overview: React.ReactNode
  /** Settings for the selected resource. */
  detail: React.ReactNode
  /** Accessible heading for the selected resource. */
  title: React.ReactNode
  /** Optional context shown below the detail heading. */
  description?: React.ReactNode
  /** Visible label for the back action. Defaults to “Back”. */
  backLabel?: React.ReactNode
  /** More specific screen-reader label for the back action. */
  backAriaLabel?: string
  onBack: () => void
  /** Disable when the consumer owns a more specific deep-link focus target. */
  autoFocusDetail?: boolean
}
/** One-level settings navigation with focus transfer and restoration. Mark a preferred surviving overview target with `data-settings-deck-fallback`. */
export declare const SettingsDeck: React.ComponentType<
  SettingsDeckProps & React.RefAttributes<HTMLDivElement>
>

export interface SettingsSectionProps extends Omit<React.HTMLAttributes<HTMLElement>, 'title'> {
  title?: React.ReactNode
  description?: React.ReactNode
  action?: React.ReactNode
}
/** A titled settings group with an optional section-level action. */
export declare const SettingsSection: React.ComponentType<SettingsSectionProps & React.RefAttributes<HTMLElement>>

/** A bordered group of related settings rows. */
export declare const SettingsList: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement> & React.RefAttributes<HTMLDivElement>
>

export type SettingsRowLayout = 'auto' | 'inline' | 'stacked'
export interface SettingsRowProps extends Omit<React.HTMLAttributes<HTMLDivElement>, 'children'> {
  label: React.ReactNode
  description?: React.ReactNode
  meta?: React.ReactNode
  /** Primary interactive or value control on the trailing edge. */
  control?: React.ReactNode
  /** Optional secondary action, composed after `control`. */
  action?: React.ReactNode
  /** `auto` stacks the trailing slot in narrow containers. */
  layout?: SettingsRowLayout
}
/** A key/value settings row with a responsive trailing control/action slot. */
export declare const SettingsRow: React.ComponentType<SettingsRowProps & React.RefAttributes<HTMLDivElement>>

export interface SettingsFieldControlProps {
  id: string
  name?: string
  'data-field'?: string
  'aria-invalid'?: true
  'aria-describedby'?: string
}
export type SettingsFieldControlSize = 'fit' | 'compact' | 'default' | 'full'
export interface SettingsFieldProps
  extends Omit<SettingsRowProps, 'label' | 'description' | 'meta' | 'control'> {
  id?: string
  name?: string
  /** Stable configuration path used by global-settings deep links. */
  field?: string
  label: React.ReactNode
  description?: React.ReactNode
  meta?: React.ReactNode
  error?: React.ReactNode
  controlSize?: SettingsFieldControlSize
  renderControl: (props: SettingsFieldControlProps) => React.ReactNode
}
/** Labelled settings row with generated IDs, validation ARIA, and standard widths. */
export declare const SettingsField: React.ComponentType<
  SettingsFieldProps & React.RefAttributes<HTMLDivElement>
>

export declare const Skeleton: React.ComponentType<React.HTMLAttributes<HTMLSpanElement>>

export interface StatusDotProps extends React.HTMLAttributes<HTMLSpanElement> {
  tone?: 'accent' | 'alert' | 'warn' | 'ink'
  pulse?: boolean
}
export declare const StatusDot: React.ComponentType<StatusDotProps>

export interface StatusPanelProps {
  variant?: 'info' | 'success' | 'warn' | 'alert'
  icon?: React.ReactNode
  headline: React.ReactNode
  detail?: React.ReactNode
  className?: string
}
export declare const StatusPanel: React.ComponentType<StatusPanelProps>

export interface TabsProps extends Omit<React.HTMLAttributes<HTMLDivElement>, 'dir'> {
  value?: string
  defaultValue?: string
  onValueChange?(value: string): void
  orientation?: 'horizontal' | 'vertical'
  dir?: 'ltr' | 'rtl'
}
export declare const Tabs: React.ComponentType<TabsProps>
export interface TabsListProps extends React.HTMLAttributes<HTMLDivElement> {
  variant?: 'line'
}
export declare const TabsList: React.ComponentType<TabsListProps>
export interface TabsTriggerProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  value: string
  /** Defaults to a semantic 16px icon inferred from `value`; `false` hides it. */
  icon?: React.ReactNode | false
}
export declare const TabsTrigger: React.ComponentType<TabsTriggerProps>
export interface TabsContentProps extends React.HTMLAttributes<HTMLDivElement> {
  value: string
}
export declare const TabsContent: React.ComponentType<TabsContentProps>

export interface TerminalCommandLineProps {
  /** The command, verbatim — also the hover title and the copy payload. */
  command: string
  /** Prompt glyph, accent ink. Default `'$'`. */
  prompt?: string
  /** Trailing slot for header chips / pills. */
  chips?: React.ReactNode
  /** Render the copy affordance (standard copied/failed flash; the
      clipboard write survives insecure origins). */
  copy?: boolean
  className?: string
}
/** One-line command header — the `$ command` row a terminal-shaped card
    opens with: accent prompt glyph, mono command that ellipsizes with the
    full text riding on `title`, optional trailing chips and copy button.
    Like `CodeEditor`, never carry a private command-line header in a
    worker asset; import this instead. */
export declare const TerminalCommandLine: React.ComponentType<TerminalCommandLineProps>

export interface TerminalStreamProps {
  /** Pane label (`stdout`, `stderr`, `build`), rendered uppercase. */
  label: string
  /** The stream body; renders nothing when empty — the caller decides
      what "no output" should say, if anything. */
  text: string
  /** `'err'` tints the body warn, and nothing more: stderr is the user's
      program failing, not the console failing. Default `'out'`. */
  tone?: 'out' | 'err'
  /** Parse ANSI SGR colors in the body (`AnsiText`); plain otherwise. */
  ansi?: boolean
  /** Collapse behind the expand toggle past this many lines (default 12)… */
  clampLines?: number
  /** …or this many characters (default 2000). */
  clampChars?: number
  className?: string
}
/** Labeled monospace stream pane — the shared rendering for stdout /
    stderr / log bodies in terminal-shaped cards: whitespace preserved,
    long output clamped behind an `expand · N lines / collapse` toggle,
    scrolls within itself, never page-wide. Like `CodeEditor`, never carry
    a private stream pane in a worker asset; import this instead. */
export declare const TerminalStream: React.ComponentType<TerminalStreamProps>

/** The console app provides the Radix `TooltipProvider`; compose Root/Trigger/Content only. */
export interface TooltipProps {
  open?: boolean
  defaultOpen?: boolean
  onOpenChange?(open: boolean): void
  delayDuration?: number
  children?: React.ReactNode
}
export declare const Tooltip: React.ComponentType<TooltipProps>
export interface TooltipTriggerProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  asChild?: boolean
}
export declare const TooltipTrigger: React.ComponentType<TooltipTriggerProps>
export interface TooltipContentProps extends React.HTMLAttributes<HTMLDivElement> {
  side?: 'top' | 'right' | 'bottom' | 'left'
  align?: 'start' | 'center' | 'end'
  sideOffset?: number
  collisionPadding?: number
  avoidCollisions?: boolean
}
export declare const TooltipContent: React.ComponentType<TooltipContentProps>

export interface CodeEditorHandle {
  focus(): void
  /** Put the cursor on `line` (1-based, clamped), scroll the pane that holds
      the editor so the line sits centered, and focus. Before the editor
      mounts the request is kept and replayed once it does. */
  revealLine(line: number, column?: number): void
}
export interface CodeEditorProps {
  value: string
  onChange(next: string): void
  /** Monaco language id (`'markdown'`, `'json'`, `'yaml'`, …); unknown ids render plain. */
  language: string
  /** Class for the outer wrapper (borders, min-height, width). */
  className?: string
  placeholder?: string
  /** Read-only: content stays selectable/copyable, chrome unchanged. */
  readOnly?: boolean
  /** Inert and dimmed (implies read-only). */
  disabled?: boolean
  autoFocus?: boolean
  id?: string
  'aria-label'?: string
  /** Observes keys bubbling out of the editor (shortcuts like ⌘S) — keys
      the editor consumes for editing never reach it. */
  onKeyDown?: React.KeyboardEventHandler<HTMLDivElement>
  /** Domain identifiers to autocomplete (e.g. SQL table/column names +
      keywords). Non-empty turns on the as-you-type suggest popup and
      registers a completion provider for the current `language`. */
  completions?: readonly string[]
}
/** The console's Monaco-backed code editor — the one editor for every code
    or long-text editing surface, themed by the console's design tokens in
    both themes. Grows with content — put it inside an `overflow-auto` pane.
    Never bundle `monaco-editor` (or any other editor) into a worker asset;
    import this instead. */
export declare const CodeEditor: React.ComponentType<CodeEditorProps & React.RefAttributes<CodeEditorHandle>>

export interface CodeHighlightProps {
  code: string
  /** Prism language id (`'javascript'`, `'python'`, …); unknown ids render plain. */
  language: string
  className?: string
  /** Wrap long lines (`whitespace-pre-wrap`) instead of preserving strict pre. */
  wrap?: boolean
}
export declare const CodeHighlight: React.ComponentType<CodeHighlightProps>

export interface JsonHighlightProps {
  code: string
  className?: string
  wrap?: boolean
}
export declare const JsonHighlight: React.ComponentType<JsonHighlightProps>

export interface MarkdownProps {
  children: string
  className?: string
}
export declare const Markdown: React.ComponentType<MarkdownProps>

export interface MarkdownPreviewProps {
  markdown: string
  className?: string
}
/** `Markdown` inside the standard `bg-bg` pane chrome — the preview
    counterpart to `CodeEditor` for markdown-editing UIs. */
export declare const MarkdownPreview: React.ComponentType<MarkdownPreviewProps>

export type ModelId = string
export type ThinkingLevel = string
export interface ReasoningEffortOption {
  effort: string
  description?: string
}
export interface ModelOption {
  id: ModelId
  label: string
  contextWindow?: number
  supportsThinking?: boolean
  supportsVision?: boolean
  reasoningEfforts?: ReasoningEffortOption[]
}
export interface ModelPickerProps {
  value: ModelId | null
  options: ModelOption[]
  openRequest?: number
  thinkingLevel: ThinkingLevel
  onChange: (next: ModelId) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  disabled?: boolean
  loading?: boolean
  showRefresh?: boolean
  placeholder?: string
  showProviderConfiguration?: boolean
  showReasoningEffort?: boolean
  className?: string
}
/** The Console's responsive searchable model catalog picker. */
export declare const ModelPicker: React.ComponentType<ModelPickerProps>

export interface WorkerConfigurationDialogProps {
  /** Which worker to open in global Settings; `null` does nothing. */
  configurationId: string | null
  onClose: () => void
}
/**
 * Compatibility bridge for older worker bundles. It forwards to the global
 * Settings modal and renders no local dialog. New pages should declare
 * `configurationId` in `host.pages.register` instead.
 */
export declare const WorkerConfigurationDialog: React.ComponentType<WorkerConfigurationDialogProps>

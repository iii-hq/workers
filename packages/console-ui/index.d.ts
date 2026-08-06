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
  on<P = unknown>(
    functionId: string,
    handler: (payload: P) => void | Promise<void>,
  ): () => void
  registerTrigger(input: {
    type: string
    function_id: string
    config: Record<string, unknown>
  }): () => void
  addConnectionStateListener(handler: (state: unknown) => void): () => void
}

/* ── slot contracts ─────────────────────────────────────────────────── */

/**
 * Where the workspace pane hosting the page sits. `'right'` only for the
 * rightmost column of a multi-column tab — a single-column tab is `'left'`,
 * so pages can treat `'left'` as the default orientation.
 */
export type PanelSide = 'left' | 'right'

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
}

export interface PageRegistration {
  /** kebab-case, unique per tab; convention `<worker>-<name>`. Routes at `#/ext/<id>`. */
  id: string
  /** Nav label. */
  title: string
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
  isMatch(functionId: string): boolean
  tryRender(message: FunctionTriggerMessage): React.ReactNode | null
  tryRenderRunning?(message: FunctionTriggerMessage): React.ReactNode | null
  tryRenderPreview?(message: FunctionTriggerMessage): React.ReactNode | null
  FunctionIdLabel?: React.ComponentType<{ functionId: string }>
  primaryTabLabel?: string
}

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue }

/**
 * Props for a configuration-form override (the `configForms` slot). The
 * override replaces the form region only — dirty tracking, save/reset, and
 * error mapping stay host-owned.
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

/**
 * What `setup(host)` receives. Every registrar returns an unregister fn AND
 * is auto-tracked: the loader runs all of them on dispose.
 */
export interface Host {
  iii: ExtensionIii
  /** The curated component record — same components as the named exports below. */
  components: Record<string, React.ComponentType<any>>
  useTheme(): 'light' | 'dark'
  /** The script's asset path, e.g. `state/page.js`. */
  path: string
  pages: { register(page: PageRegistration): () => void }
  functionTriggers: {
    register(renderer: FunctionTriggerRenderer): () => void
  }
  configForms: {
    register(
      configurationId: string,
      component: React.ComponentType<ConfigFormProps>,
    ): () => void
  }
}

/** The ONLY required export of a script asset. */
export type SetupFn = (
  host: Host,
) => void | (() => void) | Promise<void | (() => void)>

/* ── module-level api (same objects the Host carries) ───────────────── */

export declare const iii: ExtensionIii
export declare const components: Record<string, React.ComponentType<any>>
export declare function useTheme(): 'light' | 'dark'
/** Design-token names, for documentation/tooling; styling just uses `var(--color-*)`. */
export declare const tokens: readonly string[]

/* ── the shared component library ───────────────────────────────────── */

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  variant?: 'default' | 'warn' | 'alert' | 'accent'
}
export declare const Badge: React.ComponentType<BadgeProps>

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'primary' | 'ghost' | 'pill' | 'icon' | 'terminal' | 'wiggle'
  size?: 'sm' | 'md' | 'lg' | 'icon'
  /** Render as the child element (Radix Slot) instead of a `<button>`. */
  asChild?: boolean
}
export declare const Button: React.ComponentType<
  ButtonProps & React.RefAttributes<HTMLButtonElement>
>

/** Root is state-only; compose with `DialogTrigger`/`DialogContent`. */
export interface DialogProps {
  open?: boolean
  defaultOpen?: boolean
  onOpenChange?(open: boolean): void
  modal?: boolean
  children?: React.ReactNode
}
export declare const Dialog: React.ComponentType<DialogProps>
export interface DialogTriggerProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  asChild?: boolean
}
export declare const DialogTrigger: React.ComponentType<DialogTriggerProps>
export declare const DialogClose: React.ComponentType<DialogTriggerProps>
/** Portalled, centered, with overlay and close affordance built in. */
export declare const DialogContent: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement>
>
export declare const DialogTitle: React.ComponentType<
  React.HTMLAttributes<HTMLHeadingElement>
>
export declare const DialogDescription: React.ComponentType<
  React.HTMLAttributes<HTMLParagraphElement>
>

export interface DropdownMenuProps {
  open?: boolean
  defaultOpen?: boolean
  onOpenChange?(open: boolean): void
  modal?: boolean
  children?: React.ReactNode
}
export declare const DropdownMenu: React.ComponentType<DropdownMenuProps>
export interface DropdownMenuTriggerProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  asChild?: boolean
}
export declare const DropdownMenuTrigger: React.ComponentType<DropdownMenuTriggerProps>
export interface DropdownMenuContentProps
  extends React.HTMLAttributes<HTMLDivElement> {
  side?: 'top' | 'right' | 'bottom' | 'left'
  align?: 'start' | 'center' | 'end'
  sideOffset?: number
}
export declare const DropdownMenuContent: React.ComponentType<DropdownMenuContentProps>
export interface DropdownMenuItemProps
  extends React.HTMLAttributes<HTMLDivElement> {
  disabled?: boolean
  onSelect?(event: Event): void
}
export declare const DropdownMenuItem: React.ComponentType<DropdownMenuItemProps>
export declare const DropdownMenuLabel: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement>
>
export declare const DropdownMenuSeparator: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement>
>

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
export interface FileDiffProps {
  /** Pass empty `contents` for a created (old) / deleted (new) file. */
  oldFile: FileDiffSide
  newFile: FileDiffSide
  diffStyle?: 'unified' | 'split'
  /** Long lines wrap by default; `'scroll'` preserves strict columns. */
  overflow?: 'scroll' | 'wrap'
  className?: string
}
/** The console's one file-diff surface — the diff is computed from the two
    full file bodies and rendered by the console's bundled diff engine,
    following the active theme. Like `CodeEditor`, never bundle a diff
    renderer into a worker asset; import this instead. */
export declare const FileDiff: React.ComponentType<FileDiffProps>

/** Controlled string input (`onChange` receives the value, not the event). */
export interface InputProps
  extends Omit<
    React.InputHTMLAttributes<HTMLInputElement>,
    'onChange' | 'value'
  > {
  value: string
  onChange: (next: string) => void
  /** Opt out of the default `lowercase` text-transform (verbatim values: keys, URLs). */
  preserveCase?: boolean
}
export declare const Input: React.ComponentType<
  InputProps & React.RefAttributes<HTMLInputElement>
>

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

export interface PageShellProps
  extends React.HTMLAttributes<HTMLDivElement> {}
/** The pane's root column — fills the pane, `--color-panel` background. */
export declare const PageShell: React.ComponentType<PageShellProps>

export interface PageHeaderProps {
  /** Identity glyph, rendered at 16px in faint ink (any svg fits). */
  icon?: React.ReactNode
  /** The page's name — console chrome vocabulary: mono, lowercase. */
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
  /** Column width in px (fixed — navigation stays put while main flexes). */
  width?: number
}
/** The navigation column: slightly gray, fixed width, own scroll. */
export declare const PageSidebar: React.ComponentType<PageSidebarProps>

/** The primary workspace column: `--color-panel`, takes what's left. */
export declare const PageMain: React.ComponentType<
  React.HTMLAttributes<HTMLElement>
>

export interface SelectOption<T extends string = string> {
  value: T
  label: string
  /** Optional hover tooltip on the option row. */
  title?: string
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
  'aria-label'?: string
  'aria-busy'?: boolean
  placeholder?: string
  /** Render a leading option that clears the selection (calls `onClear`, not `onChange`). */
  allowEmpty?: boolean
  emptyLabel?: string
  onClear?: () => void
  renderGroupHeader?: (group: SelectGroup<T>) => React.ReactNode
}
export declare const Select: <T extends string = string>(
  props: SelectProps<T>,
) => React.ReactNode

export declare const Skeleton: React.ComponentType<
  React.HTMLAttributes<HTMLSpanElement>
>

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

export interface TabsProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, 'dir'> {
  value?: string
  defaultValue?: string
  onValueChange?(value: string): void
  orientation?: 'horizontal' | 'vertical'
  dir?: 'ltr' | 'rtl'
}
export declare const Tabs: React.ComponentType<TabsProps>
export declare const TabsList: React.ComponentType<
  React.HTMLAttributes<HTMLDivElement>
>
export interface TabsTriggerProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  value: string
}
export declare const TabsTrigger: React.ComponentType<TabsTriggerProps>
export interface TabsContentProps extends React.HTMLAttributes<HTMLDivElement> {
  value: string
}
export declare const TabsContent: React.ComponentType<TabsContentProps>

/** The console app provides the Radix `TooltipProvider`; compose Root/Trigger/Content only. */
export interface TooltipProps {
  open?: boolean
  defaultOpen?: boolean
  onOpenChange?(open: boolean): void
  delayDuration?: number
  children?: React.ReactNode
}
export declare const Tooltip: React.ComponentType<TooltipProps>
export interface TooltipTriggerProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  asChild?: boolean
}
export declare const TooltipTrigger: React.ComponentType<TooltipTriggerProps>
export interface TooltipContentProps
  extends React.HTMLAttributes<HTMLDivElement> {
  side?: 'top' | 'right' | 'bottom' | 'left'
  align?: 'start' | 'center' | 'end'
  sideOffset?: number
}
export declare const TooltipContent: React.ComponentType<TooltipContentProps>

export interface CodeEditorHandle {
  focus(): void
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
}
/** The console's Monaco-backed code editor — the one editor for every code
    or long-text editing surface, themed by the console's design tokens in
    both themes. Grows with content — put it inside an `overflow-auto` pane.
    Never bundle `monaco-editor` (or any other editor) into a worker asset;
    import this instead. */
export declare const CodeEditor: React.ComponentType<
  CodeEditorProps & React.RefAttributes<CodeEditorHandle>
>

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

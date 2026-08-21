/**
 * Injectable console UI — the contract between the console SPA and
 * worker-shipped UI scripts (iii/tech-specs/2026-07-17-injectable-ui).
 *
 * A `console:script` asset is an ESM module whose default export is
 * `setup(host)`. Everything the script registers goes through `host` so the
 * loader can dispose exactly what the script contributed on hot reload,
 * delete, or worker disconnect. These types are mirrored by the workspace
 * package `@iii-dev/console-ui` (packages/console-ui), which is what worker
 * UI projects link against; the conformance test holds the two together.
 */

import type { UiClasses } from '@iii-dev/console-ui/ui-classes'
import type { IIIConnectionState, RegisterTriggerInput } from '@/lib/iii-client'
import type { FunctionTriggerMessage } from '@/types/chat'

/**
 * The SPA's live engine client, narrowed: the client is shared by the whole
 * tab (traces, chat, every extension), so a script must never be able to
 * tear it down — `dispose()` is deliberately not carried.
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
   * (`<functionId>::<browserId>`) and default `metadata.internal = true` —
   * multi-tab-safe, catalog-invisible.
   */
  on<P = unknown>(
    functionId: string,
    handler: (payload: P) => void | Promise<void>,
  ): () => void
  registerTrigger(input: RegisterTriggerInput): () => void
  addConnectionStateListener(
    handler: (state: IIIConnectionState) => void,
  ): () => void
}

/** What `@iii-dev/console-ui` (the `/vendor/console-ui.js` shim) re-exports. */
export interface ConsoleApi {
  iii: ExtensionIii
  /**
   * Curated component library — pre-styled by the console's own CSS.
   * Everything else in `src/components/ui/` stays console-private until
   * promoted deliberately.
   */
  // biome-ignore lint/suspicious/noExplicitAny: heterogeneous component props
  components: Record<string, React.ComponentType<any>>
  /** `'light' | 'dark'`, reactive (backed by `html[data-theme]`). */
  useTheme(): 'light' | 'dark'
  /** Design-token names, for documentation/tooling; styling just uses `var(--color-*)`. */
  tokens: readonly string[]
  /** Stable namespaced CSS recipes for shared visual patterns. */
  uiClasses: UiClasses
}

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
  /**
   * Latest context sent through `host.panels.open()` for this page. The event
   * id changes even when the context value is identical, so pages can respond
   * to every user action. Ephemeral: workspace persistence stores the page,
   * not this payload.
   */
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
 * A function-trigger renderer — the runtime version of the first-party
 * ToolView families. Dispatch order: injected renderers first (registration
 * order), then the first-party families, then the JSON fallback; first
 * non-null wins, `null` always means "fall through". Host semantics stay
 * host-owned: errors parse before success, the approval bar is never
 * renderer-rendered.
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
   * non-null render visible in the chat flow while the function's raw
   * request/response details remain collapsed.
   */
  metadata?: {
    display?: boolean
    /** Keep the display as the card header and expand details underneath it. */
    displayAction?: 'expand'
  }
  /**
   * Redact the raw request/response before the card DISPLAYS OR COPIES it —
   * the `raw json` tab renders `message.input` / `message.output` verbatim
   * and its copy button copies the same value, which a renderer's own card
   * cannot contain. The console applies this to both exits (see
   * `rawRedactor` in components/function-trigger/renderer-registry.tsx);
   * what counts as secret stays the worker's to declare, never the host's.
   *
   * Consulted only for messages this renderer's `isMatch` claims (first
   * claiming renderer that declares it wins), once for the request and once
   * for the response.
   *
   * Receives an arbitrary JSON-ish value and returns the redacted copy. Must
   * be pure and total: no mutation, no I/O, no throw for any shape (cycles
   * included). Called during the card's render and fenced — a throw fails
   * CLOSED to a placeholder, never back to the raw value.
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
  outcome?:
    | 'delivered'
    | 'delivery_failed'
    | 'skipped'
    | 'expired'
    | 'unregistered'
    | 'invalidated'
  retirementReason?:
    | 'once_consumed'
    | 'max_fires'
    | 'expired'
    | 'unregistered'
    | 'invalidated'
    | 'exhausted'
}

/**
 * Renders only the source-specific section of a trigger activity. Dispatch is
 * registration-ordered and `null` falls through to the next renderer before
 * the host's generic source fallback.
 */
export interface TriggerActivityRenderer {
  /** e.g. `cron/page.js#trigger-activity` */
  id: string
  isMatch(triggerType: string): boolean
  tryRender(activity: TriggerActivityMessage): React.ReactNode | null
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
 * override replaces the FORM REGION inside the workers-tab editor — dialog
 * chrome, dirty tracking, save/reset, and error mapping stay host-owned,
 * so an override cannot break persistence.
 */
export interface ConfigFormProps {
  /** Configuration id, e.g. `state`. */
  id: string
  /**
   * Deliberately wider than the structural form's schema: `null` means the
   * worker registered a configuration value but no JSON schema — the one
   * branch the built-in form cannot render at all.
   */
  schema: Record<string, unknown> | null
  value: JsonValue
  onChange(next: JsonValue): void
  /** JSON-pointer → message, merged client + server validation. */
  errors?: ReadonlyMap<string, string>
  /**
   * Deep-link focus request: the decoded fieldPath segments from
   * `#/workers/configuration/<id>/<fieldPath>`. The host's own scroll+focus
   * effect targets schema-form DOM ids only, so honoring this is the
   * override's job. Absent when the editor wasn't opened via a field link.
   */
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

/**
 * Props for a provider-specific editor inside the chat model picker. Provider
 * workers own authentication nuances while the console keeps the router
 * slice, validation, dirty tracking, and save/reset lifecycle authoritative.
 */
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

/**
 * Props a chat footer turn-summary receives. The extension owns the summary
 * data; the host supplies only the active session and its live turn state.
 */
export interface SessionTurnSummaryProps {
  sessionId: string
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

/**
 * What `setup(host)` receives. Every registrar returns an unregister fn AND
 * is auto-tracked: the loader runs all of them on dispose.
 */
export interface Host {
  iii: ExtensionIii
  components: ConsoleApi['components']
  useTheme: ConsoleApi['useTheme']
  uiClasses: ConsoleApi['uiClasses']
  /** The script's asset path, e.g. `state/page.js`. */
  path: string
  pages: {
    register(page: PageRegistration): () => void
  }
  functionTriggers: {
    register(renderer: FunctionTriggerRenderer): () => void
  }
  triggerRenderers: {
    register(renderer: TriggerActivityRenderer): () => void
  }
  panels: {
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
  providerConfigForms: {
    register(
      providerId: string,
      component: React.ComponentType<ProviderConfigFormProps>,
    ): () => void
  }
  chat: {
    registerSessionChip(chip: SessionChipRegistration): () => void
    registerTurnSummary(summary: SessionTurnSummaryRegistration): () => void
    /** Jump the sidebar to this session. Feature-detect on older consoles. */
    selectConversation?(sessionId: string): void
    /** Live composer model for a conversation, including unsaved drafts. */
    composerModel?(conversationId?: string | null): string | null
  }
}

/** The ONLY required export of a script asset. */
export type SetupFn = (
  host: Host,
) => void | (() => void) | Promise<void | (() => void)>

export type UiAssetKind = 'script' | 'style'

export interface UiAssetRef {
  path: string
  kind: UiAssetKind
  hash: string
}

/** Push payloads delivered to the tab's `iii::console::ui-assets` handler. */
export type UiAssetsPush =
  | { event: 'sync'; assets: UiAssetRef[] }
  | { event: 'set'; path: string; kind: UiAssetKind; hash: string }
  | { event: 'delete'; path: string; kind: UiAssetKind; hash: string }

declare global {
  interface Window {
    __III_CONSOLE__?: {
      React: unknown
      ReactDOM: unknown
      ReactDOMClient: unknown
      JsxRuntime: unknown
      api: ConsoleApi | null
    }
  }
}

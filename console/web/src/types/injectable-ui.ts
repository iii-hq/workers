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
}

export interface PageRegistration {
  /** kebab-case, unique per tab; convention `<worker>-<name>`. Routes at `#/ext/<id>`. */
  id: string
  /** Nav label. */
  title: string
  /** The page body (right pane). */
  render: React.ComponentType
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

/**
 * What `setup(host)` receives. Every registrar returns an unregister fn AND
 * is auto-tracked: the loader runs all of them on dispose.
 */
export interface Host {
  iii: ExtensionIii
  components: ConsoleApi['components']
  useTheme: ConsoleApi['useTheme']
  /** The script's asset path, e.g. `state/page.js`. */
  path: string
  pages: {
    register(page: PageRegistration): () => void
  }
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

import type { ExtensionIii } from './index.js'

/**
 * Container-driven narrow state: attach `ref` to the pane root; `narrow`
 * is true while its width is below `below` (default 720). Measures
 * synchronously on attach, observes resizes, ignores zero widths.
 */
export declare function useContainerNarrow(options?: { below?: number }): {
  ref: (el: HTMLElement | null) => void
  narrow: boolean
}

/** `useState` mirrored to localStorage as JSON, best effort. */
export declare function usePaneState<T>(
  key: string,
  initial: T,
): [T, (next: T | ((prev: T) => T)) => void]

export type CopyFlashState = 'idle' | 'copied' | 'failed'
/** Click-to-copy with a flash; `ms` is how long `copied`/`failed` stays up (default 1400). */
export declare function useCopyFlash(
  text: string,
  ms?: number,
): { state: CopyFlashState; copy: () => void }

export interface WorkerLiveOptions<T> {
  iii: ExtensionIii
  /** Trigger types whose events re-run `fetch`. */
  triggers: readonly string[]
  fetch: () => Promise<T>
  /** Visible-tab poll cadence while the live bindings are unavailable (default 15000). */
  pollMs?: number
  /** Tab-scoped handler id, e.g. `iii::<worker>-ui::events`; the host appends `::<browserId>`. */
  handlerId: string
}
export interface WorkerLive<T> {
  data: T | null
  loading: boolean
  error: string | null
  /** True while updates arrive through the live trigger bindings. */
  live: boolean
  refresh: () => void
}
/** `fetch` once, re-fetch on every trigger event, poll only while not live. */
export declare function useWorkerLive<T>(options: WorkerLiveOptions<T>): WorkerLive<T>

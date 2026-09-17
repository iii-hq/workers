import type * as React from 'react'
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

/** `value`, settled: updates only after `ms` without a change (default 200). */
export declare function useDebounce<T>(value: T, ms?: number): T

export interface SplitDragOptions<S> {
  /** The separator moves along the horizontal axis (else vertical). */
  horizontal: boolean
  /** Capture the drag origin from the pointer-down; `null` refuses the drag. */
  begin(event: React.PointerEvent<HTMLElement>): S | null
  /** The pointer moved `delta` px along the axis since `begin`. */
  move(origin: S, delta: number): void
  /** An arrow key along the axis: -1 towards start, +1 towards end. */
  step(direction: -1 | 1, event: React.KeyboardEvent<HTMLElement>): void
}
export interface SplitDragHandlers {
  onPointerDown: React.PointerEventHandler<HTMLElement>
  onPointerMove: React.PointerEventHandler<HTMLElement>
  onPointerUp: React.PointerEventHandler<HTMLElement>
  onPointerCancel: React.PointerEventHandler<HTMLElement>
  onLostPointerCapture: React.PointerEventHandler<HTMLElement>
  onKeyDown: React.KeyboardEventHandler<HTMLElement>
}
/**
 * Pointer + keyboard resizer for a `role="separator"`: spread the handlers
 * on the handle, keep `aria-value*` yourself. Pointer capture makes the
 * drag survive leaving the handle; arrows step -1/+1 along the axis.
 */
export declare function useSplitDrag<S>(options: SplitDragOptions<S>): SplitDragHandlers

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
  /**
   * Trigger types whose events re-run `fetch`; an object form carries the
   * binding `config` (a `stream` trigger's `stream_name`/`group_id`).
   */
  triggers: readonly (string | { type: string; config?: Record<string, unknown> })[]
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

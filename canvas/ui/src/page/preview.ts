/**
 * The mermaid preview's render state machine, extracted pure so the
 * keep-last-good-SVG behavior and stale-render fencing are testable
 * without mermaid or a DOM.
 *
 * The pipeline: every (debounced) source edit begins a render under a fresh
 * monotonic seq; the async parse/render settles later carrying the same seq.
 * A settlement for any seq but the CURRENT one is dropped, so a slow old
 * render can never overwrite a newer one. A failed parse keeps the last good
 * SVG and carries the error (with the source line mermaid names, when it
 * names one) — the preview never blanks while the user types through an
 * invalid intermediate state.
 */

export interface PreviewState {
  /** Seq of the render currently owning the preview. */
  seq: number
  /** True while the owning render has not settled. */
  rendering: boolean
  /** Last successfully rendered SVG; survives later failures. */
  svg: string | null
  /** Parse/render error of the owning render; null after a success. */
  error: string | null
  /** 1-based source line named by the parse error, when it names one. */
  errorLine: number | null
}

export const INITIAL_PREVIEW: PreviewState = {
  seq: 0,
  rendering: false,
  svg: null,
  error: null,
  errorLine: null,
}

/** A new render takes ownership; an out-of-order begin is ignored. */
export function beginRender(state: PreviewState, seq: number): PreviewState {
  if (seq <= state.seq) return state
  return { ...state, seq, rendering: true }
}

/** The owning render produced an SVG; stale settlements are dropped. */
export function renderSucceeded(
  state: PreviewState,
  seq: number,
  svg: string,
): PreviewState {
  if (seq !== state.seq) return state
  return { ...state, rendering: false, svg, error: null, errorLine: null }
}

/**
 * The owning render failed to parse; the last good SVG stays on screen and
 * the diagnostics strip gets the message. Stale settlements are dropped.
 */
export function renderFailed(
  state: PreviewState,
  seq: number,
  message: string,
): PreviewState {
  if (seq !== state.seq) return state
  return {
    ...state,
    rendering: false,
    error: message,
    errorLine: parseErrorLine(message),
  }
}

/**
 * Pull the 1-based line number out of a mermaid error message, if present
 * ("Parse error on line 3: …", "Lexical error on line 12. …").
 */
export function parseErrorLine(message: string): number | null {
  const match = /\bline\s+(\d+)/i.exec(message)
  if (!match) return null
  const line = Number(match[1])
  return Number.isFinite(line) && line > 0 ? line : null
}

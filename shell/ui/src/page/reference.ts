/* "Reference in chat": turning an editor selection into the
   `#file(path:from-to)` token the console composer understands. React-free
   so the tests can import it (see error-display.ts for the precedent). */

export interface LineRange {
  /** 1-based, inclusive. */
  from: number
  /** 1-based, inclusive; never below `from`. */
  to: number
}

interface EditorSelectionLike {
  startLine: number
  startColumn: number
  endLine: number
  endColumn: number
}

/** The lines a selection covers. A selection that ends at the very start
    of a line (a drag that stopped just past a newline, or a whole-line
    selection) does not include that line. */
export function selectionLines(selection: EditorSelectionLike): LineRange {
  const from = Math.min(selection.startLine, selection.endLine)
  let to = Math.max(selection.startLine, selection.endLine)
  const endsAtLineStart =
    selection.endLine >= selection.startLine ? selection.endColumn === 1 : selection.startColumn === 1
  if (to > from && endsAtLineStart) to -= 1
  return { from, to }
}

/** The path a mention carries: relative to the chat's folder when the file
    lives under it (what the send path reads by default), absolute when this
    pane browses somewhere else. */
export function mentionPathFor(absPath: string, workingDir: string | null): string {
  if (workingDir === null || workingDir === '') return absPath
  const base = workingDir.replace(/\/+$/, '')
  if (absPath === base) return absPath
  return absPath.startsWith(`${base}/`) ? absPath.slice(base.length + 1) : absPath
}

/** `#file(src/a.ts:12-40)`, or `#file(src/a.ts:12)` for a single line. */
export function formatFileReference(path: string, range: LineRange): string {
  const lines = range.from === range.to ? `${range.from}` : `${range.from}-${range.to}`
  return `#file(${path}:${lines})`
}

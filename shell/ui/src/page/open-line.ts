import type { DiffOp } from './diff'

export interface GutterLine {
  line: number
  side: 'old' | 'new'
}

/** Pierre renders each gutter cell as `[data-line-type][data-column-number]`
    inside the diff's shadow root; the click reaches the host with the inner
    nodes on `composedPath()`. Deleted rows number the old file. */
export function gutterLineFromPath(path: readonly EventTarget[]): GutterLine | null {
  for (const target of path) {
    const dataset = (target as { dataset?: DOMStringMap }).dataset
    const raw = dataset?.columnNumber
    if (raw === undefined) continue
    const line = Number.parseInt(raw, 10)
    if (!Number.isFinite(line) || line < 1) return null
    return { line, side: dataset?.lineType === 'deletion' ? 'old' : 'new' }
  }
  return null
}

/** The working-file line that stands where an old-file line used to be:
    the line itself when it survived, else the first line after the last
    kept line before it. */
export function newLineForOld(ops: readonly DiffOp[], oldLine: number): number {
  let lastNew = 0
  for (const op of ops) {
    if (op.type === 'same') {
      if (op.oldLine >= oldLine) return op.newLine
      lastNew = op.newLine
    } else if (op.type === 'add') {
      lastNew = op.newLine
    } else if (op.oldLine >= oldLine) {
      return Math.max(1, lastNew + 1)
    }
  }
  return Math.max(1, lastNew + 1)
}

/** First changed line in the working file, or 1 for an unchanged body. */
export function firstChangedLine(ops: readonly DiffOp[]): number {
  for (const op of ops) {
    if (op.type === 'add') return op.newLine
    if (op.type === 'del') return newLineForOld(ops, op.oldLine)
  }
  return 1
}

export function resolveEditorLine(ops: readonly DiffOp[], target: GutterLine): number {
  return target.side === 'new' ? target.line : newLineForOld(ops, target.line)
}

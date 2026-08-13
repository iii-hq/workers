/* Line-count arithmetic for the changes feed — the +N/−M chips on rows
   whose file lives outside every repo, diffed against the last content
   this page saw. */

export function splitLines(s: string): string[] {
  return s === '' ? [] : s.split('\n')
}

/** Approximate added/removed line counts: strip the common prefix and
    suffix, count what's left on each side. Exact for pure appends,
    prepends, and contiguous edits; an upper bound for interleaved ones —
    chip-grade arithmetic, not a diff. */
export function lineDelta(
  oldText: string,
  newText: string,
): { add: number; del: number } {
  const oldLines = splitLines(oldText)
  const newLines = splitLines(newText)
  let head = 0
  while (
    head < oldLines.length &&
    head < newLines.length &&
    oldLines[head] === newLines[head]
  ) {
    head++
  }
  let tail = 0
  while (
    tail < oldLines.length - head &&
    tail < newLines.length - head &&
    oldLines[oldLines.length - 1 - tail] === newLines[newLines.length - 1 - tail]
  ) {
    tail++
  }
  return { add: newLines.length - head - tail, del: oldLines.length - head - tail }
}

/* Line diff for the explorer's diff pane — computed here, rendered
   here, no shared component involved. Myers on lines (prefix/suffix
   stripped first), then context folding, then char-span pairing so a
   replaced line can emphasize what actually changed inside it. */

export type DiffOp =
  | { type: 'same'; text: string; oldLine: number; newLine: number }
  | { type: 'del'; text: string; oldLine: number; hl?: [number, number] }
  | { type: 'add'; text: string; newLine: number; hl?: [number, number] }

/** Beyond this many middle lines per side, skip the LCS and report the
    whole middle as one replace — the pane stays instant on huge files
    at the cost of a coarser diff. */
const MYERS_LIMIT = 5000

/** Myers stores one frontier snapshot per edit step for the backtrack,
    so memory is O(D·(N+M)) — a D budget caps it: two mostly different
    files fall back to the coarse replace instead of freezing the tab
    inside a synchronous useMemo. */
const MYERS_MAX_D = 400

function splitLines(text: string): string[] {
  const body = text.endsWith('\n') ? text.slice(0, -1) : text
  return body === '' ? [] : body.split('\n')
}

export function diffLines(oldText: string, newText: string): DiffOp[] {
  const a = splitLines(oldText)
  const b = splitLines(newText)

  let head = 0
  while (head < a.length && head < b.length && a[head] === b[head]) head++
  let tail = 0
  while (
    tail < a.length - head &&
    tail < b.length - head &&
    a[a.length - 1 - tail] === b[b.length - 1 - tail]
  ) {
    tail++
  }

  const ops: DiffOp[] = []
  for (let i = 0; i < head; i++) {
    ops.push({ type: 'same', text: a[i], oldLine: i + 1, newLine: i + 1 })
  }

  const aMid = a.slice(head, a.length - tail)
  const bMid = b.slice(head, b.length - tail)
  const midOps =
    aMid.length > MYERS_LIMIT || bMid.length > MYERS_LIMIT
      ? coarseReplace(aMid, bMid)
      : myers(aMid, bMid)
  for (const op of midOps) {
    if (op.type === 'same') {
      ops.push({ ...op, oldLine: op.oldLine + head, newLine: op.newLine + head })
    } else if (op.type === 'del') {
      ops.push({ ...op, oldLine: op.oldLine + head })
    } else {
      ops.push({ ...op, newLine: op.newLine + head })
    }
  }

  for (let i = 0; i < tail; i++) {
    const oldLine = a.length - tail + i + 1
    const newLine = b.length - tail + i + 1
    ops.push({ type: 'same', text: a[oldLine - 1], oldLine, newLine })
  }

  markIntraline(ops)
  return ops
}

function coarseReplace(a: string[], b: string[]): DiffOp[] {
  const ops: DiffOp[] = []
  for (let i = 0; i < a.length; i++) ops.push({ type: 'del', text: a[i], oldLine: i + 1 })
  for (let i = 0; i < b.length; i++) ops.push({ type: 'add', text: b[i], newLine: i + 1 })
  return ops
}

/** Classic O((N+M)·D) Myers over the stripped middle; emits del-before-
    add within each changed region. */
function myers(a: string[], b: string[]): DiffOp[] {
  const n = a.length
  const m = b.length
  if (n === 0 && m === 0) return []
  const max = n + m
  const offset = max
  const v = new Int32Array(2 * max + 1)
  const trace: Int32Array[] = []

  outer: for (let d = 0; d <= max; d++) {
    if (d > MYERS_MAX_D) return coarseReplace(a, b)
    trace.push(v.slice())
    for (let k = -d; k <= d; k += 2) {
      let x: number
      if (k === -d || (k !== d && v[offset + k - 1] < v[offset + k + 1])) {
        x = v[offset + k + 1]
      } else {
        x = v[offset + k - 1] + 1
      }
      let y = x - k
      while (x < n && y < m && a[x] === b[y]) {
        x++
        y++
      }
      v[offset + k] = x
      if (x >= n && y >= m) break outer
    }
  }

  // Backtrack from (n, m) through the recorded frontiers.
  const rev: DiffOp[] = []
  let x = n
  let y = m
  for (let d = trace.length - 1; d > 0; d--) {
    const prev = trace[d]
    const k = x - y
    let prevK: number
    if (k === -d || (k !== d && prev[offset + k - 1] < prev[offset + k + 1])) {
      prevK = k + 1
    } else {
      prevK = k - 1
    }
    const prevX = prev[offset + prevK]
    const prevY = prevX - prevK
    while (x > prevX && y > prevY) {
      rev.push({ type: 'same', text: a[x - 1], oldLine: x, newLine: y })
      x--
      y--
    }
    if (x === prevX) {
      rev.push({ type: 'add', text: b[y - 1], newLine: y })
      y--
    } else {
      rev.push({ type: 'del', text: a[x - 1], oldLine: x })
      x--
    }
  }
  while (x > 0 && y > 0) {
    rev.push({ type: 'same', text: a[x - 1], oldLine: x, newLine: y })
    x--
    y--
  }
  while (x > 0) {
    rev.push({ type: 'del', text: a[x - 1], oldLine: x })
    x--
  }
  while (y > 0) {
    rev.push({ type: 'add', text: b[y - 1], newLine: y })
    y--
  }
  rev.reverse()

  // Normalize each changed region to del-run then add-run.
  const ops: DiffOp[] = []
  let dels: DiffOp[] = []
  let adds: DiffOp[] = []
  const flush = () => {
    ops.push(...dels, ...adds)
    dels = []
    adds = []
  }
  for (const op of rev) {
    if (op.type === 'same') {
      flush()
      ops.push(op)
    } else if (op.type === 'del') {
      dels.push(op)
    } else {
      adds.push(op)
    }
  }
  flush()
  return ops
}

/** Pair the i-th deleted line with the i-th added line of each changed
    region and mark the char span that differs (common prefix/suffix
    stripped) — the "what changed inside the line" emphasis. */
function markIntraline(ops: DiffOp[]): void {
  let i = 0
  while (i < ops.length) {
    if (ops[i].type !== 'del') {
      i++
      continue
    }
    const delStart = i
    while (i < ops.length && ops[i].type === 'del') i++
    const addStart = i
    while (i < ops.length && ops[i].type === 'add') i++
    const pairs = Math.min(i - addStart, addStart - delStart)
    for (let p = 0; p < pairs; p++) {
      const del = ops[delStart + p] as Extract<DiffOp, { type: 'del' }>
      const add = ops[addStart + p] as Extract<DiffOp, { type: 'add' }>
      const [dh, ah] = charSpans(del.text, add.text)
      if (dh !== null) del.hl = dh
      if (ah !== null) add.hl = ah
    }
  }
}

function charSpans(
  oldText: string,
  newText: string,
): [[number, number] | null, [number, number] | null] {
  let head = 0
  const min = Math.min(oldText.length, newText.length)
  while (head < min && oldText[head] === newText[head]) head++
  let tail = 0
  while (
    tail < min - head &&
    oldText[oldText.length - 1 - tail] === newText[newText.length - 1 - tail]
  ) {
    tail++
  }
  const oldSpan: [number, number] = [head, oldText.length - tail]
  const newSpan: [number, number] = [head, newText.length - tail]
  return [oldSpan[0] < oldSpan[1] ? oldSpan : null, newSpan[0] < newSpan[1] ? newSpan : null]
}

export type DiffRow =
  | { kind: 'op'; op: DiffOp }
  | { kind: 'fold'; index: number; count: number; ops: DiffOp[] }

/** Fold long unmodified runs down to `context` lines on each exposed
    edge; a run at the very top keeps only its trailing context and one
    at the very bottom only its leading context. */
export function foldRows(ops: DiffOp[], context = 3): DiffRow[] {
  const rows: DiffRow[] = []
  let foldIndex = 0
  let i = 0
  while (i < ops.length) {
    if (ops[i].type !== 'same') {
      rows.push({ kind: 'op', op: ops[i] })
      i++
      continue
    }
    const start = i
    while (i < ops.length && ops[i].type === 'same') i++
    const run = ops.slice(start, i)
    const atTop = start === 0
    const atBottom = i === ops.length
    const lead = atTop ? 0 : context
    const trail = atBottom ? 0 : context
    if (run.length <= lead + trail + 1) {
      for (const op of run) rows.push({ kind: 'op', op })
      continue
    }
    for (let j = 0; j < lead; j++) rows.push({ kind: 'op', op: run[j] })
    const hidden = run.slice(lead, run.length - trail)
    rows.push({ kind: 'fold', index: foldIndex++, count: hidden.length, ops: hidden })
    for (let j = run.length - trail; j < run.length; j++) {
      rows.push({ kind: 'op', op: run[j] })
    }
  }
  return rows
}

export function diffTotals(ops: DiffOp[]): { add: number; del: number } {
  let add = 0
  let del = 0
  for (const op of ops) {
    if (op.type === 'add') add++
    else if (op.type === 'del') del++
  }
  return { add, del }
}

/**
 * Pure, dependency-free git-DAG lane assignment — the layout brain behind
 * GitGraph.tsx. Given commits newest-first (as `git log --all` emits them),
 * it assigns each commit a column (lane) and produces the parent edges the SVG
 * renderer draws. No React, no DOM: unit-tested in isolation
 * (graph-layout.test.ts, run with `npx tsx`).
 *
 * The algorithm is the standard git-log lane walk:
 *
 *   lanes[]  — one slot per active branch line; each holds the hash the lane is
 *              still waiting to reach (going downward), or null when free.
 *
 *   For each commit C, in order:
 *     1. column = the lowest lane already waiting for C.hash; if none waits for
 *        it, C is a fresh head → take the lowest free lane (or append one).
 *     2. Every lane that was waiting for C.hash has now arrived: free them all.
 *     3. For each parent P of C, open a lane that waits for P and emit an edge
 *        C→P (fromColumn = C.column, toColumn = P's lane):
 *          - reuse a lane ALREADY waiting for P if one exists (this is how two
 *            branches converge on a shared parent onto ONE column — the key to
 *            correct merge/rejoin routing);
 *          - else the FIRST parent keeps C's own column (mainline stays
 *            straight);
 *          - else (extra merge parents) take the lowest free lane / append.
 *
 * Reuse-first parent assignment guarantees every child that shares a parent
 * routes its edge to the SAME column the parent ultimately occupies (the lowest
 * lane waiting for it never moves lower while it is waited on), so edges never
 * point at the wrong endpoint.
 *
 * Pass-through verticals need no separate structure: a lane with no commit
 * between a child and its parent is exactly one edge spanning those rows, so
 * drawing every edge from child row to parent row draws all the verticals too.
 */

/** One parsed `git log` record. `time` is the committer unix time (seconds). */
export interface Commit {
  hash: string
  parents: string[]
  refs: string[]
  author: string
  time: number
  subject: string
}

export interface GraphEdge {
  fromRow: number
  fromColumn: number
  toRow: number
  toColumn: number
  /** Lane-color index (mod GRAPH_COLORS) of the branch line this edge draws. */
  color: number
  /**
   * true  → the parent lane already existed, so C's line rejoins it: bend at the
   *         BOTTOM (near the parent) and keep the branch color of `fromColumn`.
   * false → a fresh lane opened for this parent (a fan-out / new branch): bend
   *         at the TOP (near the child), branch color of `toColumn`.
   * Meaningless when fromColumn === toColumn (a straight mainline segment).
   */
  reused: boolean
  /** false when the parent falls outside the loaded window (edge runs off-bottom). */
  parentKnown: boolean
}

export interface GraphRow {
  commit: Commit
  row: number
  column: number
  /** Node color index (mod GRAPH_COLORS) = the commit's own lane. */
  color: number
  isRefTip: boolean
}

export interface GraphLayout {
  rows: GraphRow[]
  edges: GraphEdge[]
  /** Total column count (max column used + 1); 0 for an empty history. */
  columns: number
}

/** Number of lane colors the palette cycles through (see styles.css --gh-g0..7). */
export const GRAPH_COLORS = 8

interface RawEdge {
  fromRow: number
  fromColumn: number
  toColumn: number
  parentHash: string
  reused: boolean
}

export function layout(commits: Commit[]): GraphLayout {
  const lanes: (string | null)[] = []
  const rows: GraphRow[] = []
  const rawEdges: RawEdge[] = []
  const rowByHash = new Map<string, number>()
  let maxColumn = -1

  const firstFree = (): number => {
    const i = lanes.indexOf(null)
    return i === -1 ? lanes.length : i
  }

  for (let i = 0; i < commits.length; i++) {
    const c = commits[i]
    rowByHash.set(c.hash, i)

    // 1. column for this commit
    let col = lanes.indexOf(c.hash)
    if (col === -1) col = firstFree()

    // 2. every lane waiting for this commit has arrived — free them
    for (let j = 0; j < lanes.length; j++) {
      if (lanes[j] === c.hash) lanes[j] = null
    }

    // 3. open a lane per parent, emit an edge per parent
    for (let p = 0; p < c.parents.length; p++) {
      const ph = c.parents[p]
      const existing = lanes.indexOf(ph)
      let toColumn: number
      let reused: boolean
      if (existing !== -1) {
        toColumn = existing
        reused = true
      } else if (p === 0) {
        toColumn = col
        lanes[col] = ph
        reused = false
      } else {
        toColumn = firstFree()
        lanes[toColumn] = ph
        reused = false
      }
      rawEdges.push({ fromRow: i, fromColumn: col, toColumn, parentHash: ph, reused })
      if (toColumn > maxColumn) maxColumn = toColumn
    }

    if (col > maxColumn) maxColumn = col
    rows.push({
      commit: c,
      row: i,
      column: col,
      color: col % GRAPH_COLORS,
      isRefTip: c.refs.length > 0,
    })
  }

  const total = commits.length
  const edges: GraphEdge[] = rawEdges.map((e) => {
    const parentKnown = rowByHash.has(e.parentHash)
    // Branch lines keep their own color: a rejoin travels in `fromColumn`, a
    // fan-out (and any straight segment) travels in `toColumn`.
    const color = (e.reused ? e.fromColumn : e.toColumn) % GRAPH_COLORS
    return {
      fromRow: e.fromRow,
      fromColumn: e.fromColumn,
      toRow: parentKnown ? (rowByHash.get(e.parentHash) as number) : total,
      toColumn: e.toColumn,
      color,
      reused: e.reused,
      parentKnown,
    }
  })

  return { rows, edges, columns: maxColumn + 1 }
}

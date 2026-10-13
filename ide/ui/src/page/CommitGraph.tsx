/* One row's slice of the commit graph, drawn from the lanes
   `commit-graph.ts` laid out. Each line takes its branch's colour, and so
   do that branch's glyph in the tree and its chip in the log: one branch,
   one colour. The default branch is drawn in faint ink, and lines no branch
   named are ghost.

   The dot of a commit some worktree has checked out is a ring: ink for the
   worktree the IDE is in, the lane's colour for the others. */

import type { GraphRow } from './commit-graph'

const GLYPHS = ['blue', 'purple', 'teal', 'green', 'amber', 'rose'] as const
export type Glyph = (typeof GLYPHS)[number] | 'faint' | null

/** A branch's colour: fixed by its name, so it is the same everywhere. A
    remote branch takes its local name's (`origin/x` is `x`'s colour). */
export function glyphOf(
  name: string | null,
  defaultBranch: string | null,
  remotes: ReadonlySet<string> = new Set(),
): Glyph {
  if (name === null) return null
  const slash = name.indexOf('/')
  const local = slash > 0 && remotes.has(name.slice(0, slash)) ? name.slice(slash + 1) : name
  if (local === defaultBranch) return 'faint'
  let hash = 0
  for (let i = 0; i < local.length; i += 1) hash = (hash * 31 + local.charCodeAt(i)) >>> 0
  return GLYPHS[hash % GLYPHS.length]
}

const COLORS: Readonly<Record<NonNullable<Glyph>, string>> = {
  blue: 'var(--color-glyph-blue)',
  purple: 'var(--color-glyph-purple)',
  teal: 'var(--color-glyph-teal)',
  green: 'var(--color-glyph-green)',
  amber: 'var(--color-glyph-amber)',
  rose: 'var(--color-glyph-rose)',
  faint: 'var(--color-ink-faint)',
}

export function glyphColor(glyph: Glyph): string {
  return glyph === null ? 'var(--color-ink-ghost)' : COLORS[glyph]
}

export const LANE_WIDTH = 12
/** Lanes past this many are drawn off the cell's edge. */
export const MAX_LANES = 10

export function GraphCell({
  row,
  height,
  color,
  ring,
  lanes: fixedLanes,
  maxLanes = MAX_LANES,
}: {
  row: GraphRow
  height: number
  /** A lane's colour from its key. */
  color: (key: string | null) => string
  /** The commit is a worktree's HEAD: `here` for the IDE's own. */
  ring: 'here' | 'worktree' | null
  /** The cell's width in lanes, the same for every row of a log so the
      text after it lines up; this row's own lanes when absent. */
  lanes?: number
  /** Lanes past this many are drawn on the last one. */
  maxLanes?: number
}) {
  const lanes = fixedLanes ?? Math.min(Math.max(row.width, row.lane + 1), maxLanes)
  const width = lanes * LANE_WIDTH + 4
  const x = (lane: number) => Math.min(lane, maxLanes) * LANE_WIDTH + LANE_WIDTH / 2 + 2
  const mid = height / 2
  // A line that changes lane bends over the half-row it crosses.
  const line = (from: number, to: number, y0: number, y1: number) =>
    from === to
      ? `M${x(from)} ${y0}V${y1}`
      : `M${x(from)} ${y0}C${x(from)} ${(y0 + y1) / 2} ${x(to)} ${(y0 + y1) / 2} ${x(to)} ${y1}`
  const dot = color(row.key)
  return (
    // lint-allow no-inline-svg: drawn data (the lanes of the history), not an icon
    <svg className="shui-git-graph" width={width} height={height} viewBox={`0 0 ${width} ${height}`} aria-hidden="true">
      {row.up.map((edge, index) => (
        <path key={`u${index}`} d={line(edge.from, edge.to, 0, mid)} stroke={color(edge.key)} />
      ))}
      {row.down.map((edge, index) => (
        <path key={`d${index}`} d={line(edge.from, edge.to, mid, height)} stroke={color(edge.key)} />
      ))}
      {ring === null ? (
        <circle cx={x(row.lane)} cy={mid} r={3} fill={dot} />
      ) : (
        <circle
          className="shui-git-graph-ring"
          cx={x(row.lane)}
          cy={mid}
          r={4.5}
          stroke={ring === 'here' ? 'var(--color-ink)' : dot}
        />
      )}
    </svg>
  )
}

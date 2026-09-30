/* The lanes of a commit graph, laid out one page of `git log --topo-order`
   at a time: each commit sits in the lane that was waiting for it (a tip
   opens a new one), its first parent keeps that lane, and any other parent
   joins a lane already waiting for it or opens its own. Lanes never slide
   sideways and freed slots are reused, so a page continues from the state
   the previous one left, and two lines waiting for the same commit meet at
   it, as `git log --graph` draws them. */

/** A line inside one row: from lane `from` to lane `to`. `key` names what
    the line belongs to (a branch), so the view can colour it; null when
    nothing named opened it. */
export interface Edge {
  from: number
  to: number
  key: string | null
}

export interface GraphRow {
  /** The lane the commit's dot sits in. */
  lane: number
  key: string | null
  /** Lines from the row's top to its middle: every lane open above, and
      the ones waiting for this commit bend into its lane. */
  up: Edge[]
  /** Lines from the middle to the bottom: lanes passing by, and one per
      parent from the commit's lane to the lane that parent waits in. */
  down: Edge[]
  /** Lanes the row spans. */
  width: number
}

/** What a page leaves for the next: the commit each lane waits for (null
    for a free slot) and each lane's key. */
export interface GraphState {
  lanes: (string | null)[]
  keys: (string | null)[]
}

export const emptyGraphState: GraphState = { lanes: [], keys: [] }

/** Lays `commits` out below `prev`. `keyOf` names a commit that opens a
    lane (a branch tip, or a merged parent), so the lane takes its colour.
    O(commits × lanes). */
export function layoutPage(
  prev: GraphState,
  commits: readonly { sha: string; parents: readonly string[] }[],
  keyOf: (sha: string) => string | null = () => null,
): { state: GraphState; rows: GraphRow[] } {
  const lanes = [...prev.lanes]
  const keys = [...prev.keys]
  const rows: GraphRow[] = []
  const open = (sha: string): number => {
    let at = lanes.indexOf(null)
    if (at < 0) at = lanes.length
    lanes[at] = sha
    keys[at] = keyOf(sha)
    return at
  }
  for (const commit of commits) {
    const before = [...lanes]
    let lane = before.indexOf(commit.sha)
    if (lane < 0) lane = open(commit.sha)
    const key = keys[lane]
    const up: Edge[] = []
    const down: Edge[] = []
    before.forEach((waiting, at) => {
      if (waiting === null) return
      up.push({ from: at, to: waiting === commit.sha ? lane : at, key: keys[at] })
      if (waiting !== commit.sha) down.push({ from: at, to: at, key: keys[at] })
    })
    // Every lane that waited for this commit has arrived.
    lanes.forEach((waiting, at) => {
      if (waiting === commit.sha) lanes[at] = null
    })
    commit.parents.forEach((parent, n) => {
      let to = lane
      if (n === 0) {
        lanes[lane] = parent
        keys[lane] = key
      } else {
        to = lanes.indexOf(parent)
        if (to < 0) to = open(parent)
      }
      down.push({ from: lane, to, key: keys[to] })
    })
    while (lanes.length > 0 && lanes[lanes.length - 1] === null) {
      lanes.pop()
      keys.pop()
    }
    rows.push({ lane, key, up, down, width: Math.max(before.length, lanes.length, lane + 1) })
  }
  return { state: { lanes, keys }, rows }
}

import { describe, expect, it } from 'vitest'
import { emptyGraphState, type GraphRow, layoutPage } from '../commit-graph'

type Commit = { sha: string; parents: string[] }
const c = (sha: string, ...parents: string[]): Commit => ({ sha, parents })
const lanes = (rows: GraphRow[]) => rows.map((row) => row.lane)

/** Every line leaving a row at the bottom enters the next at the top. */
function joined(rows: GraphRow[]): boolean {
  for (let i = 0; i + 1 < rows.length; i += 1) {
    const below = rows[i].down.map((edge) => edge.to).sort()
    const above = [...new Set(rows[i + 1].up.map((edge) => edge.from))].sort()
    if (JSON.stringify([...new Set(below)].sort()) !== JSON.stringify(above)) return false
  }
  return true
}

describe('layoutPage', () => {
  it('keeps a linear history in one lane, ending at the root', () => {
    const { rows, state } = layoutPage(emptyGraphState, [c('c', 'b'), c('b', 'a'), c('a')])
    expect(lanes(rows)).toEqual([0, 0, 0])
    expect(rows[2].down).toEqual([])
    expect(state.lanes).toEqual([])
  })

  it('forks a merge into a second lane and brings it back at the fork point', () => {
    // m merges f into main; f and main both come from base.
    const { rows } = layoutPage(emptyGraphState, [c('m', 'x', 'f'), c('f', 'base'), c('x', 'base'), c('base')])
    expect(lanes(rows)).toEqual([0, 1, 0, 0])
    expect(rows[0].down).toEqual([
      { from: 0, to: 0, key: null },
      { from: 0, to: 1, key: null },
    ])
    // At base both lines meet: lane 1 bends into lane 0.
    expect(rows[3].up.map(({ from, to }) => [from, to])).toEqual([
      [0, 0],
      [1, 0],
    ])
    expect(joined(rows)).toBe(true)
  })

  it('opens a lane per tip and names it after the tip', () => {
    const keys: Record<string, string> = { t1: 'feat', t2: 'main' }
    const { rows } = layoutPage(
      emptyGraphState,
      [c('t1', 'base'), c('t2', 'base'), c('base')],
      (sha) => keys[sha] ?? null,
    )
    expect(lanes(rows)).toEqual([0, 1, 0])
    expect(rows.map((row) => row.key)).toEqual(['feat', 'main', 'feat'])
    expect(joined(rows)).toBe(true)
  })

  it('lays out an octopus merge and a merge into a lane already waiting', () => {
    const octopus = layoutPage(emptyGraphState, [c('o', 'a', 'b', 'd'), c('a', 'r'), c('b', 'r'), c('d', 'r'), c('r')])
    expect(lanes(octopus.rows)).toEqual([0, 0, 1, 2, 0])
    expect(joined(octopus.rows)).toBe(true)
    // t2 merges p, which the first tip's lane already waits for.
    const waiting = layoutPage(emptyGraphState, [c('t1', 'p'), c('t2', 'q', 'p'), c('q', 'p'), c('p')])
    expect(waiting.rows[1].down).toContainEqual({ from: 1, to: 0, key: null })
    expect(joined(waiting.rows)).toBe(true)
  })

  it('lays out in pages exactly as in one pass, on random histories', () => {
    let seed = 7
    const random = () => {
      seed = (seed * 1103515245 + 12345) % 2 ** 31
      return seed / 2 ** 31
    }
    for (let round = 0; round < 25; round += 1) {
      // Commits in topological order, newest first: parents come later.
      const count = 40 + Math.floor(random() * 40)
      const shas = Array.from({ length: count }, (_, i) => `c${round}-${i}`)
      const commits: Commit[] = shas.map((sha, i) => {
        const later = shas.slice(i + 1)
        if (later.length === 0) return c(sha)
        const parents = [later[Math.floor(random() * Math.min(3, later.length))]]
        if (random() < 0.25) parents.push(later[Math.floor(random() * later.length)])
        return c(sha, ...new Set(parents))
      })
      const whole = layoutPage(emptyGraphState, commits).rows
      let state = emptyGraphState
      const paged: GraphRow[] = []
      for (let at = 0; at < commits.length; at += 7) {
        const page = layoutPage(state, commits.slice(at, at + 7))
        state = page.state
        paged.push(...page.rows)
      }
      expect(paged).toEqual(whole)
      expect(joined(whole)).toBe(true)
    }
  })
})

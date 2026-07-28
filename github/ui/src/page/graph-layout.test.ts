/**
 * Unit tests for the pure lane-assignment layout. Run standalone:
 *
 *   npx tsx src/page/graph-layout.test.ts
 *
 * No test runner and no node builtins (the UI tsconfig ships no @types/node):
 * a failed assertion throws (tsx exits non-zero), a clean run logs and exits 0.
 * esbuild ignores this file — it is not a build entry point.
 */

import { type Commit, type GraphEdge, layout } from './graph-layout'

let checks = 0

function assert(cond: boolean, msg: string): void {
  checks++
  if (!cond) throw new Error(`assertion failed: ${msg}`)
}

function eq<T>(actual: T, expected: T, msg: string): void {
  assert(
    JSON.stringify(actual) === JSON.stringify(expected),
    `${msg} — expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
  )
}

function commit(hash: string, parents: string[]): Commit {
  return { hash, parents, refs: [], author: 'a', time: 0, subject: hash }
}

/** Columns in row order. */
function columns(commits: Commit[]): number[] {
  return layout(commits).rows.map((r) => r.column)
}

/** Edges leaving a given row, sorted by target column for stable comparison. */
function edgesFrom(edges: GraphEdge[], row: number): GraphEdge[] {
  return edges.filter((e) => e.fromRow === row).sort((a, b) => a.toColumn - b.toColumn)
}

/* ── 1. linear history: every commit on column 0 ─────────────────────────── */
{
  // A → B → C (each the child of the next; C is the root)
  const commits = [commit('A', ['B']), commit('B', ['C']), commit('C', [])]
  const l = layout(commits)
  eq(columns(commits), [0, 0, 0], 'linear: all columns 0')
  eq(l.columns, 1, 'linear: one column wide')
  assert(
    l.edges.every((e) => e.fromColumn === 0 && e.toColumn === 0),
    'linear: every edge stays on column 0',
  )
  eq(l.edges.length, 2, 'linear: two edges (A→B, B→C)')
  console.log('ok  linear')
}

/* ── 2. one branch + merge: split to column 1, rejoin to column 0 ────────── */
{
  // M is a merge of mainline A and feature F; both fork from branch point B.
  //   M (parents A, F)
  //   A (parent B)          mainline after the fork
  //   F (parent B)          feature commit
  //   B (parent C)          fork point, two children (A and F)
  //   C (root)
  const commits = [commit('M', ['A', 'F']), commit('A', ['B']), commit('F', ['B']), commit('B', ['C']), commit('C', [])]
  const l = layout(commits)
  eq(columns(commits), [0, 0, 1, 0, 0], 'branch+merge: F splits to column 1, rest column 0')
  eq(l.columns, 2, 'branch+merge: two columns wide')

  // M fans out to two parents: mainline A on col 0, feature F on col 1.
  const mEdges = edgesFrom(l.edges, 0)
  eq(mEdges.length, 2, 'branch+merge: merge commit has two parent edges')
  eq(
    mEdges.map((e) => e.toColumn),
    [0, 1],
    'branch+merge: merge edges land on columns 0 and 1',
  )
  eq(mEdges[1].fromColumn, 0, 'branch+merge: feature edge leaves column 0')
  assert(mEdges[1].reused === false, 'branch+merge: feature fan-out opens a fresh lane')

  // F rejoins the mainline: its parent B is already awaited on column 0.
  const fEdges = edgesFrom(l.edges, 2)
  eq(fEdges.length, 1, 'branch+merge: F has one parent edge')
  eq(fEdges[0].fromColumn, 1, 'branch+merge: F rejoin leaves column 1')
  eq(fEdges[0].toColumn, 0, 'branch+merge: F rejoin lands on column 0')
  assert(fEdges[0].reused === true, 'branch+merge: F rejoin reuses the mainline lane')
  console.log('ok  branch+merge')
}

/* ── 3. octopus: a 3-parent merge fans to columns 0, 1, 2 and rejoins ────── */
{
  //   O (parents P1, P2, P3)   octopus merge
  //   P1 (parent R)
  //   P2 (parent R)
  //   P3 (parent R)
  //   R  (root; the shared parent of all three)
  const commits = [
    commit('O', ['P1', 'P2', 'P3']),
    commit('P1', ['R']),
    commit('P2', ['R']),
    commit('P3', ['R']),
    commit('R', []),
  ]
  const l = layout(commits)
  eq(columns(commits), [0, 0, 1, 2, 0], 'octopus: three parents fan to columns 0,1,2')
  eq(l.columns, 3, 'octopus: three columns wide')

  const oEdges = edgesFrom(l.edges, 0)
  eq(oEdges.length, 3, 'octopus: merge has three parent edges')
  eq(
    oEdges.map((e) => e.toColumn),
    [0, 1, 2],
    'octopus: parent edges land on columns 0, 1, 2',
  )
  assert(
    oEdges.every((e) => e.fromColumn === 0),
    'octopus: all fan-out edges leave column 0',
  )

  // All three P* rejoin onto R at column 0.
  for (const row of [1, 2, 3]) {
    const e = edgesFrom(l.edges, row)
    eq(e.length, 1, `octopus: P at row ${row} has one edge`)
    eq(e[0].toColumn, 0, `octopus: P at row ${row} rejoins column 0`)
  }
  console.log('ok  octopus')
}

/* ── 4. unknown parents (beyond the window) run off the bottom ───────────── */
{
  // A's parent Z is not in the loaded set: the edge is marked not-known and
  // targets the bottom row so the renderer runs it off-screen.
  const commits = [commit('A', ['Z'])]
  const l = layout(commits)
  eq(l.edges.length, 1, 'window: one edge')
  assert(l.edges[0].parentKnown === false, 'window: missing parent flagged not-known')
  eq(l.edges[0].toRow, 1, 'window: unknown parent edge runs to the bottom row')
  console.log('ok  windowed history')
}

console.log(`\nall graph-layout tests passed (${checks} assertions)`)

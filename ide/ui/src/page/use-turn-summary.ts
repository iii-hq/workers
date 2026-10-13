/* The chat footer's "N files changed +x -y" pill for the newest turn.
   Totals come from the same loader the diff tabs use, computed for a
   bounded number of files so a turn of hundreds of writes does not stall
   the page; the rest of the rows stay "pending" and the pill says so.
   A row's totals hold until its file is written or the turn's record
   changes, so a disk burst reads again only the files it wrote. */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useMemo, useRef, useState } from 'react'
import { joinPath } from './coder'
import { diffLines, diffTotals } from './diff'
import { loadTurnDiff, preImageBody, turnFileFor } from './diff-load'
import type { ShellReviewFileSummary } from './review-summary-store'
import { relativeToRoot, type SessionTurn, type SessionTurnSummary } from './turns'

const SUMMARY_MAX_FILES = 40
const SUMMARY_CONCURRENCY = 3

/** Whether a row's totals hold until its file is written or the turn's
    record changes. Not when the turn kept no body from before it: the last
    commit stands in for that body, and a commit moves it without a write. */
export function totalsHold(record: SessionTurn, root: string, rel: string): boolean {
  const file = turnFileFor(record, root, rel)
  return file === null || (file.before == null && file.kind === 'created') || preImageBody(file.before) !== null
}

/** `previous` with the rows `results` has news for, or `previous` itself
    when none is news: the footer then has nothing to redraw. */
export function mergeSummaryRows(
  previous: readonly ShellReviewFileSummary[],
  results: ReadonlyMap<string, ShellReviewFileSummary>,
): readonly ShellReviewFileSummary[] {
  let changed = false
  const next = previous.map((row) => {
    const fresh = results.get(row.path)
    if (fresh === undefined || (fresh.state === row.state && fresh.add === row.add && fresh.del === row.del)) return row
    changed = true
    return fresh
  })
  return changed ? next : previous
}

/** Totals held for a file, with the record they were computed from. */
interface KnownTotals {
  record: SessionTurn
  row: ShellReviewFileSummary
}

/** The row `known` holds for `rel` under `root`, when computed from
    `record`. Totals follow the absolute path; the row takes `rel`, so a
    pane re-rooted to a parent or a child shows the path relative to it. */
export function knownRow(
  known: ReadonlyMap<string, KnownTotals>,
  record: SessionTurn,
  root: string,
  rel: string,
): ShellReviewFileSummary | null {
  const hit = known.get(joinPath(root, rel))
  if (hit === undefined || hit.record !== record) return null
  return hit.row.path === rel ? hit.row : { ...hit.row, path: rel }
}

export function useTurnSummary(
  host: Host,
  root: string | null,
  turn: SessionTurnSummary | null,
  turns: { get(turnId: string): Promise<SessionTurn | null>; forget(turnId: string): void },
  /** Bumps when the disk changed; totals of the newest turn follow. */
  refreshEpoch: number,
  /** Absolute paths written since the last pass, drained here: those files
      are read again, every other row keeps the totals it has. */
  written: Set<string>,
  /** The worker's protected paths, whose reads fail like a missing file's. */
  isProtected?: (path: string) => boolean,
): readonly ShellReviewFileSummary[] {
  const [rows, setRows] = useState<readonly ShellReviewFileSummary[]>([])
  const seqRef = useRef(0)
  const shownTurnRef = useRef<string | null>(null)
  // Totals by absolute path, for the root they were read under: writes
  // under another root never reach `written`, so a new root starts over.
  const knownRef = useRef(new Map<string, KnownTotals>())
  const knownRootRef = useRef(root)
  const turnId = turn?.turn_id ?? null
  const fileKey = useMemo(
    () => (turn ? turn.files.map((file) => `${file.path}\u0000${file.kind}`).join('\n') : ''),
    [turn],
  )

  // A running turn keeps gaining files; the record cached before they
  // existed would report them as untouched.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the file list is the trigger
  useEffect(() => {
    if (turnId !== null) turns.forget(turnId)
  }, [turnId, fileKey, turns])

  // biome-ignore lint/correctness/useExhaustiveDependencies: fileKey and refreshEpoch are the recompute triggers
  useEffect(() => {
    const seq = ++seqRef.current
    const known = knownRef.current
    for (const abs of written) known.delete(abs)
    written.clear()
    if (knownRootRef.current !== root) {
      knownRootRef.current = root
      known.clear()
    }
    if (!turn || root === null || turnId === null) {
      shownTurnRef.current = null
      known.clear()
      setRows([])
      return
    }
    const inside = turn.files
      .map((file) => ({ file, rel: relativeToRoot(file.path, root) }))
      .filter((entry): entry is { file: (typeof turn.files)[number]; rel: string } => entry.rel !== null)
    const initial: ShellReviewFileSummary[] = inside.map(({ rel }) => ({
      path: rel,
      state: 'pending',
      add: null,
      del: null,
    }))
    // Totals already on screen stay put while they recompute, so a disk
    // burst does not flip every row through "…"; another turn starts over.
    const sameTurn = shownTurnRef.current === turnId
    shownTurnRef.current = turnId
    if (!sameTurn) known.clear()
    setRows((previous) => {
      if (!sameTurn) return initial
      const shown = new Map(previous.map((row) => [row.path, row]))
      const next = initial.map((row) => shown.get(row.path) ?? row)
      return next.length === previous.length && next.every((row, index) => row === previous[index]) ? previous : next
    })
    if (inside.length === 0) return
    let cancelled = false
    void turns.get(turnId).then(async (record) => {
      if (cancelled || seqRef.current !== seq || record === null) return
      const results = new Map<string, ShellReviewFileSummary>()
      const flush = () => {
        if (!cancelled && seqRef.current === seq) setRows((previous) => mergeSummaryRows(previous, results))
      }
      const queue: typeof inside = []
      for (const entry of inside.slice(0, SUMMARY_MAX_FILES)) {
        const row = knownRow(known, record, root, entry.rel)
        if (row !== null) results.set(entry.rel, row)
        else queue.push(entry)
      }
      if (results.size > 0) flush()
      let cursor = 0
      const worker = async () => {
        while (cursor < queue.length && !cancelled) {
          const { rel } = queue[cursor++]
          let row: ShellReviewFileSummary = { path: rel, state: 'unavailable', add: null, del: null }
          let holds = false
          try {
            const contents = await loadTurnDiff(host, root, rel, record, isProtected)
            if (!contents.binary && !contents.noBaseline) {
              const totals = diffTotals(diffLines(contents.oldContents, contents.newContents))
              row = { path: rel, state: 'ready', add: totals.add, del: totals.del }
            }
            holds = totalsHold(record, root, rel)
          } catch {
            // Unavailable this pass; a read that failed is tried again.
          }
          // A pass superseded mid-read may hold a body from before a write.
          if (cancelled) return
          if (holds) known.set(joinPath(root, rel), { record, row })
          results.set(rel, row)
          flush()
        }
      }
      await Promise.all(Array.from({ length: Math.min(SUMMARY_CONCURRENCY, queue.length) }, worker))
    })
    return () => {
      cancelled = true
    }
  }, [host, root, turnId, fileKey, refreshEpoch, turns, isProtected])

  return rows
}

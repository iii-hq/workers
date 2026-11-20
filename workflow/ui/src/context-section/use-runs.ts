import type { ExtensionIii } from '@iii-dev/console-ui'
import { useEffect, useMemo, useState } from 'react'
import {
  buildRunView,
  isLiveRun,
  parseRunDetails,
  parseRunList,
  type WorkflowRunRef,
  type WorkflowRunView,
} from '../lib/run-view'

/** A run still going is read again this long after its last read came back. */
export const WORKFLOW_REFRESH_MS = 5_000
/** A run still going asks for its node details (a live read per node) at most this often. */
export const WORKFLOW_DETAILS_MS = 30_000
/** The list is asked again this often while the section is unfolded and nothing else asks it. */
export const WORKFLOW_IDLE_LIST_MS = 30_000
/** The newest runs of a chat the panel reads; older ones are only counted. */
export const WORKFLOW_RUN_LIMIT = 20
// A read with details reads each of up to 100 nodes live, 16 at a time with 5 s each.
const READ_TIMEOUT_MS = 60_000
// The list reads the chat's session tree and the worker's run records.
const LIST_TIMEOUT_MS = 15_000

type Iii = Pick<ExtensionIii, 'call'>
type Snapshot = NonNullable<ReturnType<typeof parseRunDetails>>

interface RunEntry {
  /** `null`: the worker no longer has the run (collected a day after it ended). */
  snapshot: Snapshot | null
  /** Nothing left to read. */
  final: boolean
  /** When a read with details last came back. */
  detailedAt?: number
  /** A finished run read before its usage settled: its next read is the last. */
  partial?: boolean
}

// By run id, for every panel: a run id is never reused, and the aside
// unmounts the panel on each tab switch.
const runCache = new Map<string, RunEntry>()
const inFlight = new Map<string, { details: boolean; done: Promise<void> }>()
// The runs `workflow::list` named, by chat: a reply never shows under another chat.
const listCache = new Map<string, WorkflowRunRef[]>()
const listing = new Map<string, { sent: boolean; done: Promise<void> }>()

/** Tests start from a clean slate. */
export function resetWorkflowReads() {
  runCache.clear()
  inFlight.clear()
  listCache.clear()
  listing.clear()
}

function isDue(entry: RunEntry | undefined, open: boolean): boolean {
  if (!entry) return true
  if (!entry.snapshot || entry.final) return false
  return open || isLiveRun(entry.snapshot.status)
}

const isLive = (entry: RunEntry | undefined) => !!entry?.snapshot && isLiveRun(entry.snapshot.status)

function wantsDetails(entry: RunEntry | undefined, open: boolean): boolean {
  if (!open) return false
  if (!entry?.snapshot || !isLiveRun(entry.snapshot.status)) return true
  return entry.detailedAt === undefined || Date.now() - entry.detailedAt >= WORKFLOW_DETAILS_MS
}

function store(runId: string, snapshot: Snapshot | null, details: boolean) {
  if (!snapshot) {
    runCache.set(runId, { snapshot: null, final: true })
    return
  }
  const prior = runCache.get(runId)
  const live = isLiveRun(snapshot.status)
  if (details) {
    // harness::metrics answers `complete: false` until a node's turns settle,
    // and a timed-out live read leaves `usage` out.
    const settled = Object.values(snapshot.node_details ?? {}).every(
      (node) => node.started_at == null || node.usage?.complete === true,
    )
    runCache.set(runId, {
      snapshot,
      final: !live && (settled || !!prior?.partial),
      detailedAt: Date.now(),
      partial: !live && !settled,
    })
    return
  }
  // A read without details keeps the last node details, their states brought up to date.
  const known = prior?.snapshot
  const nodeDetails = known?.node_details
  const grew = !!known && Object.keys(snapshot.nodes).some((uid) => !(uid in known.nodes))
  runCache.set(runId, {
    snapshot: nodeDetails
      ? {
          ...snapshot,
          node_details: Object.fromEntries(
            Object.entries(nodeDetails).map(([uid, node]) => [
              uid,
              { ...node, state: snapshot.nodes[uid] ?? node.state },
            ]),
          ),
        }
      : snapshot,
    final: false,
    // New loop passes or fanout items ask for details at the next read.
    detailedAt: grew ? undefined : prior?.detailedAt,
  })
}

function readRun(iii: Iii, runId: string, details: boolean): Promise<void> {
  const pending = inFlight.get(runId)
  // A read on its way serves this one too, unless it leaves out the details this one needs.
  if (pending && (pending.details || !details)) return pending.done
  // An upgrade waits for the read on its way, so its older reply never lands last.
  const done = (pending?.done ?? Promise.resolve())
    .then(() => iii.call('workflow::status', { run_id: runId, details }, { timeoutMs: READ_TIMEOUT_MS }))
    // A failed call is not kept: the next read asks again.
    .then(
      (reply) => store(runId, parseRunDetails(reply), details),
      () => {},
    )
    .finally(() => {
      if (inFlight.get(runId)?.done === done) inFlight.delete(runId)
    })
  inFlight.set(runId, { details, done })
  return done
}

function readList(iii: Iii, sessionId: string): Promise<void> {
  const pending = listing.get(sessionId)
  // A list not sent yet serves this one too; one on its way may predate a run
  // started since, so this one is sent after it.
  if (pending && !pending.sent) return pending.done
  const entry = { sent: false, done: Promise.resolve() }
  entry.done = (pending?.done ?? Promise.resolve())
    .then(() => {
      entry.sent = true
      return iii.call('workflow::list', { caller_session_id: sessionId }, { timeoutMs: LIST_TIMEOUT_MS })
    })
    // A failed or unreadable list keeps the runs already listed.
    .then(
      (reply) => {
        const refs = parseRunList(reply)
        if (refs) listCache.set(sessionId, refs)
      },
      () => {},
    )
    .finally(() => {
      if (listing.get(sessionId) === entry) listing.delete(sessionId)
    })
  listing.set(sessionId, entry)
  return entry.done
}

/**
 * The runs a chat and its sub-agents started, newest first: listed by
 * `workflow::list`, each read from `workflow::status`. The list is asked
 * again when the panel comes into sight, at each turn's start and end, when
 * the window gets focus, every few seconds while a turn or a run is going,
 * and every half minute otherwise while unfolded. A finished run is read until its figures settle; a run still going
 * is read again every few seconds while the section is in sight, with its
 * per-node detail (a live read per node) at a slower pace. Folded, the
 * section only needs each run's status, so the detail is left out.
 */
export function useWorkflowRuns({
  iii,
  sessionId,
  active,
  open,
  isStreaming,
}: {
  iii: Iii
  sessionId: string
  /** The panel is in sight. */
  active: boolean
  /** The section is unfolded. */
  open: boolean
  /** A turn is running: its start and end list again at once. */
  isStreaming: boolean
}): { runs: WorkflowRunView[]; earlier: number } {
  const [version, setVersion] = useState(0)
  const [tick, setTick] = useState(0)
  const [listTick, setListTick] = useState(0)

  // open, isStreaming and listTick are re-list tokens: unfolding lists at once.
  useEffect(() => {
    if (!active) return
    // A list that lands out of sight must still reach the view.
    void readList(iii, sessionId).then(() => setVersion((n) => n + 1))
  }, [iii, active, open, sessionId, isStreaming, listTick])

  useEffect(() => {
    if (!active) return
    const onFocus = () => setListTick((n) => n + 1)
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [active])

  // A sub-agent may start a run after the turn that spawned it ended. Each
  // re-list restarts this timer, so it fires only when nothing else re-lists.
  // shortcut: polls, so such a run shows up to 30 s late; drop it if workflow pushes run starts.
  // isStreaming and listTick are re-arm tokens.
  useEffect(() => {
    if (!active || !open) return
    const timer = window.setTimeout(() => setListTick((n) => n + 1), WORKFLOW_IDLE_LIST_MS)
    return () => window.clearTimeout(timer)
  }, [active, open, sessionId, isStreaming, listTick])

  // version marks a change of the list cache.
  const all = useMemo(() => listCache.get(sessionId) ?? [], [sessionId, version])
  const refs = useMemo(() => all.slice(0, WORKFLOW_RUN_LIMIT), [all])
  const runKey = refs.map((ref) => ref.runId).join()

  // runKey stands for refs; isStreaming and tick are re-read tokens.
  useEffect(() => {
    if (!active) return
    const due = refs.map((ref) => ref.runId).filter((id) => isDue(runCache.get(id), open))
    if (due.length === 0 && !isStreaming) return
    let alive = true
    let timer: number | undefined
    void Promise.all(
      due.map((id) =>
        // A read that lands out of sight must still reach the view.
        readRun(iii, id, wantsDetails(runCache.get(id), open)).then(() => setVersion((n) => n + 1)),
      ),
    ).then(() => {
      if (!alive) return
      // A turn going may start a run, and a run going may start another.
      const relist = isStreaming || refs.some((ref) => isLive(runCache.get(ref.runId)))
      // The next read waits for these, so a slow engine is never asked twice
      // at once; armed from the cache, so a failed read retries and a run the
      // worker no longer has does not.
      if (relist || due.some((id) => isDue(runCache.get(id), open)))
        timer = window.setTimeout(() => {
          setTick((n) => n + 1)
          if (relist) setListTick((n) => n + 1)
        }, WORKFLOW_REFRESH_MS)
    })
    return () => {
      alive = false
      window.clearTimeout(timer)
    }
  }, [iii, active, open, runKey, isStreaming, tick])

  // version marks a change of the run cache.
  const runs = useMemo(
    () =>
      refs.flatMap((ref) => {
        const snapshot = runCache.get(ref.runId)?.snapshot
        return snapshot ? [buildRunView(ref, snapshot)] : []
      }),
    [refs, version],
  )
  return { runs, earlier: all.length - refs.length }
}

import { useEffect, useMemo, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import type { Conversation, Message } from '@/types/chat'
import {
  buildRunView,
  collectWorkflowRuns,
  isLiveRun,
  parseRunDetails,
  type WorkflowRunView,
} from './workflow/run-view'

/** A run still going is read again this long after its last read came back. */
export const WORKFLOW_REFRESH_MS = 5_000
/** A run still going asks for its node details (a live read per node) at most this often. */
export const WORKFLOW_DETAILS_MS = 30_000
/** The newest runs of a chat the panel reads; older ones are only counted. */
export const WORKFLOW_RUN_LIMIT = 20
// A read with details reads each of up to 100 nodes live, 16 at a time with 5 s each.
const READ_TIMEOUT_MS = 60_000

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

/** Tests start from a clean slate. */
export function resetWorkflowReads() {
  runCache.clear()
  inFlight.clear()
}

function isDue(entry: RunEntry | undefined, open: boolean): boolean {
  if (!entry) return true
  if (!entry.snapshot || entry.final) return false
  return open || isLiveRun(entry.snapshot.status)
}

function wantsDetails(entry: RunEntry | undefined, open: boolean): boolean {
  if (!open) return false
  if (!entry?.snapshot || !isLiveRun(entry.snapshot.status)) return true
  return (
    entry.detailedAt === undefined ||
    Date.now() - entry.detailedAt >= WORKFLOW_DETAILS_MS
  )
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
  const grew =
    !!known && Object.keys(snapshot.nodes).some((uid) => !(uid in known.nodes))
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

function readRun(runId: string, details: boolean): Promise<void> {
  const pending = inFlight.get(runId)
  // A read on its way serves this one too, unless it leaves out the details this one needs.
  if (pending && (pending.details || !details)) return pending.done
  const done = getIiiClient()
    .then((client) =>
      client.call(
        'workflow::status',
        { run_id: runId, details },
        { timeoutMs: READ_TIMEOUT_MS },
      ),
    )
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

/**
 * The runs a chat started, newest first, read from `workflow::status`. A
 * finished run is read until its figures settle; a run still going is read
 * again every few seconds while the section is in sight, with its per-node
 * detail (a live read per node) at a slower pace. Folded, the section only
 * needs each run's status, so the detail is left out.
 */
export function useContextWorkflow({
  messages,
  sessions,
  active,
  shown,
  open,
  status,
}: {
  messages: readonly Message[]
  /** The chat's descendants: node sessions name their run. */
  sessions: readonly Conversation[]
  /** The panel is in sight. */
  active: boolean
  /** The section is not hidden in the Sections menu. */
  shown: boolean
  /** The section is unfolded. */
  open: boolean
  /** The chat's status: a turn's end retries a read that failed. */
  status?: string
}): { runs: WorkflowRunView[]; earlier: number } {
  const all = useMemo(
    () => collectWorkflowRuns(messages, sessions),
    [messages, sessions],
  )
  const refs = useMemo(() => all.slice(-WORKFLOW_RUN_LIMIT).reverse(), [all])
  const [version, setVersion] = useState(0)
  const [tick, setTick] = useState(0)
  const runKey = refs.map((ref) => ref.runId).join()

  // biome-ignore lint/correctness/useExhaustiveDependencies: runKey stands for refs; status and tick are re-read tokens
  useEffect(() => {
    if (!active || !shown) return
    const due = refs
      .map((ref) => ref.runId)
      .filter((id) => isDue(runCache.get(id), open))
    if (due.length === 0) return
    let alive = true
    let timer: number | undefined
    void Promise.all(
      due.map((id) =>
        readRun(id, wantsDetails(runCache.get(id), open)).then(() => {
          if (alive) setVersion((n) => n + 1)
        }),
      ),
    ).then(() => {
      // The next read waits for these, so a slow engine is never asked twice
      // at once; armed from the cache, so a failed read of a known run retries.
      if (
        alive &&
        due.some((id) => {
          const entry = runCache.get(id)
          return !!entry?.snapshot && isDue(entry, open)
        })
      )
        timer = window.setTimeout(
          () => setTick((n) => n + 1),
          WORKFLOW_REFRESH_MS,
        )
    })
    return () => {
      alive = false
      window.clearTimeout(timer)
    }
  }, [active, shown, open, runKey, status, tick])

  // biome-ignore lint/correctness/useExhaustiveDependencies: version marks a change of the cache
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

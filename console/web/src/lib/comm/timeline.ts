import type { CommEvent } from '@/types/iii-agent-event'

/**
 * Merge history + live comm events. `seq` is authoritative for sequenced
 * events (incoming wins on collision); events with `seq === 0` (live events
 * whose durable append failed) are interleaved by `at` without ever
 * reordering sequenced events.
 */
export function mergeEvents(
  existing: CommEvent[],
  incoming: CommEvent[],
): CommEvent[] {
  const bySeq = new Map<number, CommEvent>()
  const unsequenced: CommEvent[] = []
  for (const e of [...existing, ...incoming]) {
    if (e.seq === 0) unsequenced.push(e)
    else bySeq.set(e.seq, e)
  }
  const spine = [...bySeq.values()].sort((a, b) => a.seq - b.seq)
  unsequenced.sort((a, b) => a.at - b.at)
  for (const e of unsequenced) {
    const i = spine.findIndex((s) => s.at > e.at)
    if (i === -1) spine.push(e)
    else spine.splice(i, 0, e)
  }
  return spine
}

/** Lane order: root first, then sessions in order of first appearance. */
export function buildLanes(rootId: string, events: CommEvent[]): string[] {
  const lanes: string[] = [rootId]
  const push = (id?: string) => {
    if (id && !lanes.includes(id)) lanes.push(id)
  }
  for (const e of events) {
    push(e.from?.session_id)
    push(e.to?.session_id)
    push(e.trigger?.child_session_id)
  }
  return lanes
}

/** Walk `parentOf` links to the family root; cycle-safe. */
export function resolveRootId(
  sessionId: string,
  parentOf: (id: string) => string | null | undefined,
): string {
  let cur = sessionId
  const seen = new Set<string>()
  while (!seen.has(cur)) {
    seen.add(cur)
    const p = parentOf(cur)
    if (!p) return cur
    cur = p
  }
  return sessionId
}

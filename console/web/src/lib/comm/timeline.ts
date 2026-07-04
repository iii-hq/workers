import type { CommEvent } from '@/types/iii-agent-event'

/**
 * Merge history + live comm events. `seq` is the identity (incoming wins on
 * collision); events with `seq === 0` (live events whose durable append
 * failed) have no identity — they are all kept and interleaved by `at`.
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
  return [...bySeq.values(), ...unsequenced].sort((a, b) =>
    a.seq !== 0 && b.seq !== 0 ? a.seq - b.seq : a.at - b.at,
  )
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

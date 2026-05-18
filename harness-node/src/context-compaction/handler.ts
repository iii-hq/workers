/**
 * agent::events stream subscriber. Mirrors
 * `context-compaction/src/lib.rs::handle_event` plus the SSE envelope
 * decoding helpers (camelCase + snake_case envelope shapes).
 */

export function extractEventPayload(
  payload: unknown,
): { session_id: string; event: unknown } | null {
  if (!payload || typeof payload !== 'object') return null;
  const obj = payload as Record<string, unknown>;
  const session_id =
    (typeof obj.groupId === 'string' && obj.groupId) ||
    (typeof obj.group_id === 'string' && obj.group_id) ||
    null;
  if (!session_id) return null;
  let event: unknown = null;
  if (
    obj.event &&
    typeof obj.event === 'object' &&
    'data' in (obj.event as Record<string, unknown>)
  ) {
    event = (obj.event as Record<string, unknown>).data;
  } else if ('data' in obj) {
    event = obj.data;
  }
  return { session_id, event };
}

export function turnEndUsage(event: unknown): Record<string, unknown> | null {
  if (!event || typeof event !== 'object') return null;
  const obj = event as Record<string, unknown>;
  const kind = typeof obj.type === 'string' ? obj.type : null;
  if (kind !== 'TurnEnd' && kind !== 'turn_end') return null;
  const msg = obj.message as Record<string, unknown> | undefined;
  const usage = msg?.usage;
  if (!usage || typeof usage !== 'object') return null;
  return usage as Record<string, unknown>;
}

export function usageTotal(usage: Record<string, unknown>): number {
  const num = (k: string) => (typeof usage[k] === 'number' ? (usage[k] as number) : 0);
  // cache_write is excluded — counts toward cost but not transcript size.
  return num('input') + num('output') + num('cache_read');
}

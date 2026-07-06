/**
 * RPC adapters for the shell worker's undo journal
 * (`coder::checkpoints` / `coder::undo`). The shell journals every file
 * mutation; these read the log and reverse it, scoped to the conversation's
 * working directory (`fs_scope.root`). Mirrors the `filesystem-grants` adapter
 * shape: a thin wire layer plus an injectable `trigger` for tests.
 *
 * Undoing an `coder::undo` record is a redo — the journal treats the undo as
 * just another mutation, so replaying it restores the reverted files.
 */

import { getIiiClient } from '@/lib/iii-client'

export const CHECKPOINTS_FUNCTION_ID = 'coder::checkpoints'
export const UNDO_FUNCTION_ID = 'coder::undo'

export interface CheckpointRecord {
  seq: number
  ts: number
  sessionId?: string
  turnId?: string
  functionId: string
  files: string[]
}

export interface CheckpointsResult {
  records: CheckpointRecord[]
  truncated: boolean
}

export interface UndoRecord {
  seq: number
  functionId: string
  restored: string[]
  removed: string[]
  skipped: string[]
}

type TriggerFn = (
  functionId: string,
  payload: Record<string, unknown>,
) => Promise<unknown>

function defaultTrigger(): TriggerFn {
  return async (functionId, payload) => {
    const client = await getIiiClient()
    return client.trigger(functionId, payload)
  }
}

/** The coder undo/checkpoint functions take the full fs_scope shape. */
function fsScope(
  root: string,
  turnId?: string,
): { root: string; grants: string[]; turn_id?: string } {
  return { root, grants: [], ...(turnId ? { turn_id: turnId } : {}) }
}

/**
 * Attribution stamp for dialog-initiated undos: the shell journals the undo
 * under this id, so the resulting revert record can itself be targeted by
 * turn (= redo works on every revert row, not just the newest).
 */
const consoleUndoTurnId = (): string =>
  `console-undo-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`

const strArray = (v: unknown): string[] =>
  Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []

function coerceRecord(raw: unknown): CheckpointRecord | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  if (typeof r.seq !== 'number' || typeof r.function_id !== 'string')
    return null
  return {
    seq: r.seq,
    ts: typeof r.ts === 'number' ? r.ts : 0,
    ...(typeof r.session_id === 'string' ? { sessionId: r.session_id } : {}),
    ...(typeof r.turn_id === 'string' ? { turnId: r.turn_id } : {}),
    functionId: r.function_id,
    files: strArray(r.files),
  }
}

function coerceUndo(raw: unknown): UndoRecord | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  if (typeof r.seq !== 'number' || typeof r.function_id !== 'string')
    return null
  return {
    seq: r.seq,
    functionId: r.function_id,
    restored: strArray(r.restored),
    removed: strArray(r.removed),
    skipped: strArray(r.skipped),
  }
}

/** Journal records for `root`, newest-first (the wire contract). */
export async function listCheckpoints(
  root: string,
  opts?: { limit?: number; trigger?: TriggerFn },
): Promise<CheckpointsResult> {
  const call = opts?.trigger ?? defaultTrigger()
  const raw = (await call(CHECKPOINTS_FUNCTION_ID, {
    fs_scope: fsScope(root),
    ...(opts?.limit ? { limit: opts.limit } : {}),
  })) as { records?: unknown[]; truncated?: unknown } | null
  const records = Array.isArray(raw?.records)
    ? raw.records
        .map(coerceRecord)
        .filter((r): r is CheckpointRecord => r !== null)
    : []
  return { records, truncated: raw?.truncated === true }
}

/**
 * Reverse a turn (`turnId`) or the newest `steps` records. The two are mutually
 * exclusive; the shell rejects passing both.
 */
export async function undoCheckpoint(
  root: string,
  opts: {
    turnId?: string
    steps?: number
    /** Test override for the journal attribution stamp. */
    stampTurnId?: string
    trigger?: TriggerFn
  },
): Promise<UndoRecord[]> {
  const call = opts.trigger ?? defaultTrigger()
  const raw = (await call(UNDO_FUNCTION_ID, {
    ...(opts.turnId ? { turn_id: opts.turnId } : {}),
    ...(opts.steps ? { steps: opts.steps } : {}),
    fs_scope: fsScope(root, opts.stampTurnId ?? consoleUndoTurnId()),
  })) as { undone?: unknown[] } | null
  return Array.isArray(raw?.undone)
    ? raw.undone.map(coerceUndo).filter((r): r is UndoRecord => r !== null)
    : []
}

export interface CheckpointGroup {
  /** Stable react key: the turn id, or `seq-<n>` for turn-less records. */
  key: string
  turnId?: string
  /** Newest record's timestamp (records arrive newest-first). */
  ts: number
  functionIds: string[]
  /** Union of every file touched across the group's records. */
  files: string[]
  /** True when the group reverts a prior change (`coder::undo`) — shows "redo". */
  isRevert: boolean
  records: CheckpointRecord[]
}

const uniq = (values: string[]): string[] => [...new Set(values)]

/**
 * Fold newest-first records into per-turn groups. Records without a turn id
 * each stand alone; contiguous records sharing a turn id merge into one group.
 */
export function groupCheckpoints(
  records: CheckpointRecord[],
): CheckpointGroup[] {
  const buckets: CheckpointRecord[][] = []
  for (const rec of records) {
    const last = buckets[buckets.length - 1]
    if (rec.turnId && last && last[0].turnId === rec.turnId) {
      buckets[buckets.length - 1] = [...last, rec]
    } else {
      buckets.push([rec])
    }
  }
  return buckets.map((recs) => ({
    key: recs[0].turnId ?? `seq-${recs[0].seq}`,
    turnId: recs[0].turnId,
    ts: recs[0].ts,
    functionIds: uniq(recs.map((r) => r.functionId)),
    files: uniq(recs.flatMap((r) => r.files)),
    isRevert: recs.some((r) => r.functionId === UNDO_FUNCTION_ID),
    records: recs,
  }))
}

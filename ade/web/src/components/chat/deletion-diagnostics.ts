import type { DeletionBlocker } from '@/lib/sessions/delete-tree'

/** Presentation limits only: never used to determine force eligibility. */
export const DIAGNOSTIC_PAGE_SIZE = 24

export const blockerLabels: Record<DeletionBlocker['kind'], string> = {
  active_processing: 'Active processing',
  unconfirmed_cancellation: 'Unconfirmed cancellation',
  unknown_completion: 'Unknown completion after timeout',
}

export function observedTimestamp(value: number | undefined): string {
  if (value === undefined) return 'Not reported'
  const date = new Date(value)
  return Number.isFinite(date.getTime())
    ? date.toISOString()
    : `Invalid timestamp (${value})`
}

export interface IndexedBlocker {
  blocker: DeletionBlocker
  index: number
}

export interface BlockerGroup {
  sessionId: string
  entries: IndexedBlocker[]
  total: number
}

/** Keep every reported blocker, including duplicate/missing call IDs. */
export function groupDeletionBlockers(
  blockers: readonly DeletionBlocker[],
  query: string,
): BlockerGroup[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
  const groups = new Map<string, BlockerGroup>()
  blockers.forEach((blocker, index) => {
    let group = groups.get(blocker.session_id)
    if (!group) {
      group = { sessionId: blocker.session_id, entries: [], total: 0 }
      groups.set(blocker.session_id, group)
    }
    group.total += 1
    const searchable = [
      blocker.session_id,
      blocker.function_id,
      blocker.call_id,
      blocker.kind,
      blockerLabels[blocker.kind],
      observedTimestamp(blocker.started_at),
    ]
      .join(' ')
      .toLowerCase()
    if (terms.every((term) => searchable.includes(term)))
      group.entries.push({ blocker, index })
  })
  return [...groups.values()].filter((group) => group.entries.length)
}

/** A page caps both rows and groups, even when each blocker has its own chat. */
export function pageBlockerGroups(
  groups: readonly BlockerGroup[],
  page: number,
): BlockerGroup[] {
  let offset = page * DIAGNOSTIC_PAGE_SIZE
  let remaining = DIAGNOSTIC_PAGE_SIZE
  const result: BlockerGroup[] = []
  for (const group of groups) {
    if (offset >= group.entries.length) {
      offset -= group.entries.length
      continue
    }
    const entries = group.entries.slice(offset, offset + remaining)
    result.push({ ...group, entries })
    remaining -= entries.length
    offset = 0
    if (!remaining) break
  }
  return result
}

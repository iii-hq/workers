export interface ShellReviewFileSummary {
  path: string
  state: 'ready' | 'pending' | 'unavailable'
  add: number | null
  del: number | null
}

export interface ShellReviewSummary {
  /** Workspace-tab id of the Shell explorer that published this snapshot. */
  sourceId: string
  turnId: string | null
  files: readonly ShellReviewFileSummary[]
}

export interface ShellReviewFileSelection {
  sourceId: string
  path: string
}

interface PublishedSummary {
  revision: number
  summary: ShellReviewSummary
}

const summariesBySession = new Map<
  string,
  Map<string, PublishedSummary>
>()
const visibleSummaries = new Map<string, ShellReviewSummary>()
const summaryListeners = new Map<string, Set<() => void>>()
const selectionListeners = new Map<
  string,
  Set<(selection: ShellReviewFileSelection) => void>
>()
let revision = 0

function emitSummary(sessionId: string) {
  for (const listener of summaryListeners.get(sessionId) ?? []) listener()
}

function latestSummary(
  sources: ReadonlyMap<string, PublishedSummary>,
): ShellReviewSummary | null {
  let latest: PublishedSummary | null = null
  for (const candidate of sources.values()) {
    if (latest === null || candidate.revision > latest.revision) {
      latest = candidate
    }
  }
  return latest?.summary ?? null
}

function updateVisibleSummary(sessionId: string) {
  const previous = visibleSummaries.get(sessionId) ?? null
  const sources = summariesBySession.get(sessionId)
  const next = sources ? latestSummary(sources) : null
  if (next === null) visibleSummaries.delete(sessionId)
  else visibleSummaries.set(sessionId, next)
  if (previous !== next) emitSummary(sessionId)
}

/** Publish one explorer's current turn snapshot for a chat session. */
export function publishShellReviewSummary(
  sessionId: string,
  summary: ShellReviewSummary,
) {
  const normalized: ShellReviewSummary = {
    sourceId: summary.sourceId,
    turnId: summary.turnId,
    // Do not retain ReviewPane's full old/new file bodies when callers pass
    // its structurally compatible summary objects.
    files: summary.files.map(({ path, state, add, del }) => ({
      path,
      state,
      add,
      del,
    })),
  }
  let sources = summariesBySession.get(sessionId)
  if (!sources) {
    sources = new Map()
    summariesBySession.set(sessionId, sources)
  }
  sources.set(summary.sourceId, { revision: ++revision, summary: normalized })
  updateVisibleSummary(sessionId)
}

/** Remove a publisher without clearing a newer sibling explorer. */
export function clearShellReviewSummary(sessionId: string, sourceId: string) {
  const sources = summariesBySession.get(sessionId)
  if (!sources?.delete(sourceId)) return
  if (sources.size === 0) summariesBySession.delete(sessionId)
  updateVisibleSummary(sessionId)
}

export function getShellReviewSummary(
  sessionId: string,
): ShellReviewSummary | null {
  return visibleSummaries.get(sessionId) ?? null
}

export function subscribeShellReviewSummary(
  sessionId: string,
  listener: () => void,
): () => void {
  let listeners = summaryListeners.get(sessionId)
  if (!listeners) {
    listeners = new Set()
    summaryListeners.set(sessionId, listeners)
  }
  listeners.add(listener)
  return () => {
    listeners?.delete(listener)
    if (listeners?.size === 0) summaryListeners.delete(sessionId)
  }
}

export function emitShellReviewFileSelection(
  sessionId: string,
  selection: ShellReviewFileSelection,
) {
  for (const listener of selectionListeners.get(sessionId) ?? []) {
    listener(selection)
  }
}

export function subscribeShellReviewFileSelection(
  sessionId: string,
  listener: (selection: ShellReviewFileSelection) => void,
): () => void {
  let listeners = selectionListeners.get(sessionId)
  if (!listeners) {
    listeners = new Set()
    selectionListeners.set(sessionId, listeners)
  }
  listeners.add(listener)
  return () => {
    listeners?.delete(listener)
    if (listeners?.size === 0) selectionListeners.delete(sessionId)
  }
}

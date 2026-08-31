import type { GitChange, GitContentSource, GitFileStatus } from './git'

/** One file changed since this working directory became active. Git is
    optional: the live watcher supplies the path and first-seen baseline
    for plain folders, temporary projects, and repositories alike. */
export interface ReviewEntry {
  path: string
  change: GitChange
  baseline?: string | null
  gitDir?: string
  before?: GitContentSource
  after?: GitContentSource
}

export interface ReviewDiff {
  change: GitChange
  baseline?: string | null
  gitDir?: string
}

export interface ReviewContentsSnapshot {
  oldContents: string
  newContents: string
  noBaseline?: true
  image?: string | null
  imageUnavailable?: true
}

export function reviewContentsRepresentChange(
  entry: ReviewEntry,
  contents: ReviewContentsSnapshot,
): boolean {
  if (contents.noBaseline) return false
  if (contents.image !== undefined || contents.imageUnavailable) return true
  if (
    entry.change.status === 'added' ||
    entry.change.status === 'untracked' ||
    entry.change.status === 'deleted' ||
    entry.change.status === 'renamed'
  ) {
    return true
  }
  return contents.oldContents !== contents.newContents
}

export function canUseGitMetadataForLiveEntry(
  baselineCaptured: boolean,
  baselineUnavailable: boolean,
  baseline: string | null | undefined,
): boolean {
  return (
    !baselineUnavailable &&
    (baselineCaptured || baseline !== undefined)
  )
}

/** Each activation gets a fresh change identity so ReviewPane re-reads the
    working copy even when the same selected row is clicked again. */
export function diffForReviewEntry(entry: ReviewEntry): ReviewDiff {
  return {
    change: { ...entry.change },
    baseline: entry.baseline,
    gitDir: entry.gitDir,
  }
}

function sameGitChange(a: GitChange, b: GitChange): boolean {
  return (
    a.path === b.path &&
    a.status === b.status &&
    a.staged === b.staged &&
    a.from === b.from
  )
}

function sameContentSource(
  a?: GitContentSource,
  b?: GitContentSource,
): boolean {
  if (a === undefined || b === undefined) return a === b
  if (a.kind !== b.kind) return false
  if (a.kind === 'empty' || b.kind === 'empty') return true
  if (a.path !== b.path) return false
  return a.kind === 'revision' && b.kind === 'revision'
    ? a.revision === b.revision
    : true
}

export function sameReviewEntry(
  a: ReviewEntry | undefined,
  b: ReviewEntry,
): boolean {
  if (a === undefined) return false
  return (
    a.path === b.path &&
    sameGitChange(a.change, b.change) &&
    a.baseline === b.baseline &&
    a.gitDir === b.gitDir &&
    sameContentSource(a.before, b.before) &&
    sameContentSource(a.after, b.after)
  )
}

export function statusForLiveKind(kind: string): GitFileStatus {
  if (kind === 'created') return 'untracked'
  if (kind === 'deleted') return 'deleted'
  return 'modified'
}

/** Merge a live file event into the current review set. Existing Git
    metadata stays authoritative, while a non-Git entry keeps the first
    baseline captured before this turn's edits. */
export function mergeReviewEntry(
  previous: ReadonlyMap<string, ReviewEntry>,
  path: string,
  kind: string,
  baseline: string | null | undefined,
  gitChange?: GitChange,
): ReadonlyMap<string, ReviewEntry> {
  const existing = previous.get(path)
  const change = gitChange
    ? { ...gitChange }
    : {
        path,
        status: statusForLiveKind(kind),
        staged: false,
      }
  const nextEntry: ReviewEntry = {
    path,
    change,
    baseline:
      existing?.baseline === undefined
        ? gitChange === undefined
          ? (baseline ?? null)
          : (baseline ??
            (gitChange.status === 'added' || gitChange.status === 'untracked' ? '' : null))
        : existing.baseline,
    gitDir: existing?.gitDir,
  }
  return new Map(previous).set(path, nextEntry)
}

/** A Git refresh enriches live rows and, when requested, also seeds
    pre-existing working-tree changes. */
export function mergeGitReviewEntries(
  previous: ReadonlyMap<string, ReviewEntry>,
  changes: readonly GitChange[],
  seed = true,
): ReadonlyMap<string, ReviewEntry> {
  if (changes.length === 0) return previous
  const next = new Map(previous)
  let changed = false
  for (const change of changes) {
    const existing = next.get(change.path)
    const renameSource = change.status === 'renamed' && change.from ? change.from : null
    const renameSourceEntry = renameSource !== null ? next.get(renameSource) : undefined
    const hasRenameSource = renameSourceEntry !== undefined
    if (!seed && existing === undefined && !hasRenameSource) continue
    if (hasRenameSource && renameSource !== null && renameSource !== change.path) {
      next.delete(renameSource)
      changed = true
    }
    if (
      !hasRenameSource &&
      existing !== undefined &&
      sameGitChange(existing.change, change)
    ) {
      continue
    }
    next.set(change.path, {
      path: change.path,
      change,
      baseline: (renameSourceEntry ?? existing)?.baseline,
      gitDir: (renameSourceEntry ?? existing)?.gitDir,
    })
    changed = true
  }
  return changed ? next : previous
}

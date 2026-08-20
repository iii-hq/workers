import {
  FileDiff,
  type FileDiffEditState,
  type Host,
} from '@iii-dev/console-ui'
import {
  ChevronRight,
  FileSymlink,
  Pencil,
  Save,
  X,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import {
  coderReadFile,
  coderReadFileBase64,
  coderWriteFile,
  joinPath,
  type ReadFileResponse,
} from './coder'
import { imageMimeFromPath } from './EditorPane'
import { FileTypeIcon } from './file-type-icon'
import {
  firstChangedLine,
  gutterLineFromPath,
  resolveEditorLine,
} from './open-line'
import { richPreviewNode } from './rich-preview'
import { diffLines, diffTotals } from './diff'
import { gitHeadBaseline, gitReadSource, gitShowHead } from './git'
import type { ReviewEntry } from './review'

export interface ReviewOptions {
  diffStyle: 'unified' | 'split'
  wordWrap: boolean
  wordDiffs: boolean
  hideWhitespace: boolean
  expandUnchanged: boolean
  richPreview: boolean
}

export interface ReviewFileSummary {
  path: string
  /** Null totals are intentional: the state says whether hydration is still
      pending or failed, so consumers never present unknown work as +0/-0. */
  state: 'ready' | 'pending' | 'unavailable'
  add: number | null
  del: number | null
  oldContents: string | null
  newContents: string | null
}

export interface ReviewEditDraft {
  savedContent: string
  draft: string
  revision?: string
  mode?: number | null
}

interface ReviewPaneProps {
  host: Host
  root: string
  entries: readonly ReviewEntry[]
  activePath: string | null
  options: ReviewOptions
  collapseEpoch: number
  expandEpoch: number
  refreshEpoch: number
  onActivate: (path: string) => void
  onRequestEdit: (path: string) => boolean
  onEditDraftChange: (path: string, draft: ReviewEditDraft | null) => void
  onEditDirtyChange: (path: string, dirty: boolean) => void
  onEditSavingChange: (path: string, saving: boolean) => void
  onFileSaved: (path: string, contents: string, revision?: string) => void
  onSummaryChange: (files: readonly ReviewFileSummary[]) => void
  /** Open the working file in an editor tab with the cursor on `line`. */
  onOpenLine?: (path: string, line: number) => void
}

type FileState =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'error'; message: string }
  | ({ phase: 'ready' } & ReviewContents)

interface FileLoadDescriptor {
  host: Host
  root: string
  refreshEpoch: number
  entry: ReviewEntry
}

interface CachedFileState extends FileLoadDescriptor {
  state: FileState
}

export const LARGE_REVIEW_THRESHOLD = 40
export const LARGE_REVIEW_EAGER_FILE_COUNT = 12
const MAX_CONCURRENT_FILE_LOADS = 4
const IDLE_FILE_STATE: FileState = { phase: 'idle' }

export function defaultCollapsedReviewPaths(
  entries: readonly ReviewEntry[],
): Set<string> {
  if (entries.length <= LARGE_REVIEW_THRESHOLD) return new Set()
  return new Set(
    entries
      .slice(LARGE_REVIEW_EAGER_FILE_COUNT)
      .map((entry) => entry.path),
  )
}

export function prioritizedReviewEntries(
  entries: readonly ReviewEntry[],
  activePath: string | null,
): readonly ReviewEntry[] {
  if (activePath === null) return entries
  const active = entries.find((entry) => entry.path === activePath)
  if (active === undefined || entries[0] === active) return entries
  return [active, ...entries.filter((entry) => entry !== active)]
}

export function desiredReviewEntries(
  entries: readonly ReviewEntry[],
  activePath: string | null,
  collapsed: ReadonlySet<string>,
  viewportRequestedPaths: ReadonlySet<string>,
): readonly ReviewEntry[] {
  const prioritized = prioritizedReviewEntries(entries, activePath)
  if (entries.length <= LARGE_REVIEW_THRESHOLD) return prioritized

  const eagerPaths = new Set(
    entries
      .slice(0, LARGE_REVIEW_EAGER_FILE_COUNT)
      .map((entry) => entry.path),
  )
  return prioritized.filter(
    (entry) =>
      entry.path === activePath ||
      (!collapsed.has(entry.path) &&
        (eagerPaths.has(entry.path) || viewportRequestedPaths.has(entry.path))),
  )
}

export function expandedReviewPaths(
  collapsed: ReadonlySet<string>,
  activePath: string,
): ReadonlySet<string> {
  if (!collapsed.has(activePath)) return collapsed
  const next = new Set(collapsed)
  next.delete(activePath)
  return next
}

/** Only comparisons whose right-hand side is the current workspace can be
    edited. Index, commit, and empty/deleted targets remain review-only. */
export function reviewEntryWorktreePath(entry: ReviewEntry): string | null {
  if (entry.change.status === 'deleted') return null
  if (entry.after === undefined) return entry.path
  return entry.after.kind === 'worktree' ? entry.after.path : null
}

export function updateReviewDraft(
  saving: Readonly<{ current: boolean }>,
  next: string,
  setDraft: (next: string) => void,
): void {
  if (!saving.current) setDraft(next)
}

export interface ReviewSaveBarrier {
  readonly paths: ReadonlySet<string>
  update(path: string, saving: boolean): ReadonlySet<string>
  canTransition(): boolean
  wait(): Promise<void>
}

export function createReviewSaveBarrier(): ReviewSaveBarrier {
  let paths: ReadonlySet<string> = new Set()
  let idle = Promise.resolve()
  let resolveIdle: (() => void) | null = null

  return {
    get paths() {
      return paths
    },
    update(path, saving) {
      if (paths.has(path) === saving) return paths
      const next = new Set(paths)
      if (saving) next.add(path)
      else next.delete(path)

      if (paths.size === 0 && next.size > 0) {
        idle = new Promise<void>((resolve) => {
          resolveIdle = resolve
        })
      }
      paths = next
      if (paths.size === 0 && resolveIdle !== null) {
        const resolve = resolveIdle
        resolveIdle = null
        resolve()
        idle = Promise.resolve()
      }
      return paths
    },
    canTransition() {
      return paths.size === 0
    },
    async wait() {
      while (paths.size > 0) await idle
    },
  }
}

export function runReviewTransition(
  barrier: Pick<ReviewSaveBarrier, 'canTransition'>,
  transition: () => void,
): boolean {
  if (!barrier.canTransition()) return false
  transition()
  return true
}

export async function withReviewSavePending<T>(
  path: string,
  onSavingChange: (path: string, saving: boolean) => void,
  save: () => Promise<T>,
): Promise<T> {
  onSavingChange(path, true)
  try {
    return await save()
  } finally {
    onSavingChange(path, false)
  }
}

export function reportActiveReviewDirty(
  editing: boolean,
  path: string,
  dirty: boolean,
  onChange: (path: string, dirty: boolean) => void,
): void {
  if (editing) onChange(path, dirty)
}

export function reviewEditStatus(
  editing: boolean,
  editorState: FileDiffEditState,
  saving: boolean,
): { role: 'status' | 'alert'; message: string } | null {
  if (!editing) return null
  if (saving) return { role: 'status', message: 'saving file…' }
  if (editorState === 'loading') {
    return { role: 'status', message: 'loading inline editor…' }
  }
  if (editorState === 'error') {
    return {
      role: 'alert',
      message: 'inline editor failed to load; cancel and try again',
    }
  }
  return null
}

export function orderedReviewSummaries(
  entries: readonly ReviewEntry[],
  stats: ReadonlyMap<string, ReviewFileSummary>,
  readyPaths: ReadonlySet<string>,
  unavailablePaths: ReadonlySet<string>,
): readonly ReviewFileSummary[] {
  return entries.map((entry): ReviewFileSummary => {
    const summary = stats.get(entry.path)
    if (summary !== undefined && readyPaths.has(entry.path)) return summary
    const unavailable = unavailablePaths.has(entry.path)
    return {
      path: entry.path,
      state: unavailable ? 'unavailable' : 'pending',
      add: null,
      del: null,
      oldContents: null,
      newContents: null,
    }
  })
}

function sourceKey(source: ReviewEntry['before']): string {
  if (source === undefined) return ''
  if (source.kind === 'empty') return source.kind
  if (source.kind === 'revision') {
    return `${source.kind}\0${source.revision}\0${source.path}`
  }
  return `${source.kind}\0${source.path}`
}

function sameReviewSource(left: ReviewEntry, right: ReviewEntry): boolean {
  return (
    left.path === right.path &&
    left.baseline === right.baseline &&
    left.gitDir === right.gitDir &&
    left.change.path === right.change.path &&
    left.change.from === right.change.from &&
    left.change.status === right.change.status &&
    left.change.staged === right.change.staged &&
    sourceKey(left.before) === sourceKey(right.before) &&
    sourceKey(left.after) === sourceKey(right.after)
  )
}

function sameLoadDescriptor(
  left: FileLoadDescriptor,
  right: FileLoadDescriptor,
): boolean {
  return (
    left.host === right.host &&
    left.root === right.root &&
    left.refreshEpoch === right.refreshEpoch &&
    sameReviewSource(left.entry, right.entry)
  )
}

function normalizedForWhitespace(text: string): string {
  return text
    .split('\n')
    .map((line) => line.trim())
    .join('\n')
}

function totalsFor(oldContents: string, newContents: string, hideWhitespace: boolean) {
  return diffTotals(
    diffLines(
      hideWhitespace ? normalizedForWhitespace(oldContents) : oldContents,
      hideWhitespace ? normalizedForWhitespace(newContents) : newContents,
    ),
  )
}

export function exactCoderText(out: ReadFileResponse, path: string): string {
  if (out.is_utf8 === false) throw new Error('binary file: no text diff')
  if (out.more_lines === true) throw new Error(`file read was truncated: ${path}`)
  return out.content ?? ''
}

/** Split a root-relative path into the directory to run Git in and the name
    to ask it about. Running in the file's own directory is what lets Git
    discover a repository nested under a non-repository workspace root. */
export function gitLookupTarget(
  root: string,
  path: string,
): { cwd: string; name: string } {
  const slash = path.lastIndexOf('/')
  return slash === -1
    ? { cwd: root, name: path }
    : { cwd: joinPath(root, path.slice(0, slash)), name: path.slice(slash + 1) }
}

export interface ReviewContents {
  oldContents: string
  newContents: string
  worktreeRevision?: string
  mode?: number | null
  /** Raster image rows: data URL of the working copy, `null` once deleted.
      Text fields stay empty because the bytes are not diffable. */
  image?: string | null
  /** Raster image rows whose new side is a committed revision: the bytes
      only stream as text, so the row explains instead of previewing. */
  imageUnavailable?: true
  /** Set when the old side is the last commit rather than this turn's
      pre-turn snapshot. */
  baselineSource?: 'committed'
}

/** Last-resort baseline for a row whose pre-turn body was never captured:
    the file's committed body. It is a weaker claim than the snapshot — any
    edit already uncommitted before the turn is attributed to this turn — so
    the caller labels the diff instead of presenting it as a turn diff. */
async function loadCommittedFallback(
  host: Host,
  root: string,
  entry: ReviewEntry,
): Promise<ReviewContents | null> {
  const { change } = entry
  const oldPath = change.from ?? change.path
  const target = gitLookupTarget(root, oldPath)
  const oldContents = await gitHeadBaseline(host, target.cwd, target.name)
  if (oldContents === null) return null
  if (change.status === 'deleted') {
    return { oldContents, newContents: '', baselineSource: 'committed' }
  }
  const out = await coderReadFile(host, joinPath(root, change.path))
  return {
    oldContents,
    newContents: exactCoderText(out, change.path),
    worktreeRevision: out.revision ?? undefined,
    mode: out.mode,
    baselineSource: 'committed',
  }
}

async function loadImageContents(
  host: Host,
  root: string,
  entry: ReviewEntry,
  mime: string,
): Promise<ReviewContents> {
  if (entry.change.status === 'deleted') {
    return { oldContents: '', newContents: '', image: null }
  }
  const path = reviewEntryWorktreePath(entry)
  if (path === null) return { oldContents: '', newContents: '', imageUnavailable: true }
  const out = await coderReadFileBase64(host, joinPath(root, path))
  return {
    oldContents: '',
    newContents: '',
    worktreeRevision: out.revision ?? undefined,
    mode: out.mode,
    image: `data:${mime};base64,${out.content ?? ''}`,
  }
}

export async function loadReviewContents(
  host: Host,
  root: string,
  entry: ReviewEntry,
): Promise<ReviewContents> {
  const mime = imageMimeFromPath(entry.path)
  if (mime !== null) return loadImageContents(host, root, entry, mime)
  if (entry.before !== undefined && entry.after !== undefined) {
    const oldSide = gitReadSource(host, root, entry.before)
    if (entry.after.kind === 'worktree') {
      const [oldContents, out] = await Promise.all([
        oldSide,
        coderReadFile(host, joinPath(root, entry.after.path)),
      ])
      return {
        oldContents,
        newContents: exactCoderText(out, entry.after.path),
        worktreeRevision: out.revision ?? undefined,
        mode: out.mode,
      }
    }
    const [oldContents, newContents] = await Promise.all([
      oldSide,
      gitReadSource(host, root, entry.after),
    ])
    return { oldContents, newContents }
  }
  if (entry.baseline === null) {
    const committed = await loadCommittedFallback(host, root, entry)
    if (committed !== null) return committed
    throw new Error(
      'earlier content was not captured for this turn, and this file has no committed version to compare against',
    )
  }
  const { change } = entry
  const oldPath = change.from ?? change.path
  const oldSide =
    entry.baseline !== undefined
      ? Promise.resolve(entry.baseline)
      : change.status === 'added' || change.status === 'untracked'
        ? Promise.resolve('')
        : entry.gitDir !== undefined
          ? gitShowHead(host, entry.gitDir, oldPath.slice(oldPath.lastIndexOf('/') + 1))
          : gitShowHead(host, root, oldPath)
  const newSide: Promise<{
    contents: string
    revision?: string
    mode?: number | null
  }> =
    change.status === 'deleted'
      ? Promise.resolve({ contents: '' })
      : coderReadFile(host, joinPath(root, change.path)).then((out) =>
          ({
            contents: exactCoderText(out, change.path),
            revision: out.revision ?? undefined,
            mode: out.mode,
          }),
        )
  const [oldContents, current] = await Promise.all([oldSide, newSide])
  return {
    oldContents,
    newContents: current.contents,
    worktreeRevision: current.revision,
    mode: current.mode,
  }
}

export function ReviewPane({
  host,
  root,
  entries,
  activePath,
  options,
  collapseEpoch,
  expandEpoch,
  refreshEpoch,
  onActivate,
  onRequestEdit,
  onEditDraftChange,
  onEditDirtyChange,
  onEditSavingChange,
  onFileSaved,
  onSummaryChange,
  onOpenLine,
}: ReviewPaneProps) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() =>
    defaultCollapsedReviewPaths(entries),
  )
  const [stats, setStats] = useState<ReadonlyMap<string, ReviewFileSummary>>(new Map())
  const [viewportRequestedPaths, setViewportRequestedPaths] = useState<
    ReadonlySet<string>
  >(new Set())
  const [cacheVersion, setCacheVersion] = useState(0)
  const bodyRef = useRef<HTMLDivElement>(null)
  const entriesRef = useRef(entries)
  entriesRef.current = entries
  const autoCollapsedRef = useRef(defaultCollapsedReviewPaths(entries))
  const seenPathsRef = useRef(new Set(entries.map((entry) => entry.path)))
  const wasLargeReviewRef = useRef(entries.length > LARGE_REVIEW_THRESHOLD)
  const mountedRef = useRef(true)
  const cacheRef = useRef(new Map<string, CachedFileState>())
  const availableLoadsRef = useRef(new Map<string, FileLoadDescriptor>())
  const desiredLoadsRef = useRef(new Map<string, FileLoadDescriptor>())
  const queuedLoadsRef = useRef(new Map<string, FileLoadDescriptor>())
  const activeLoadsRef = useRef(new Set<string>())
  const pumpLoadsRef = useRef<() => void>(() => {})
  const largeReview = entries.length > LARGE_REVIEW_THRESHOLD

  useEffect(
    () => {
      mountedRef.current = true
      return () => {
        mountedRef.current = false
        availableLoadsRef.current.clear()
        desiredLoadsRef.current.clear()
        queuedLoadsRef.current.clear()
      }
    },
    [],
  )

  const bumpCache = useCallback(() => {
    if (mountedRef.current) setCacheVersion((value) => value + 1)
  }, [])

  const pumpLoads = useCallback(() => {
    while (
      activeLoadsRef.current.size < MAX_CONCURRENT_FILE_LOADS &&
      queuedLoadsRef.current.size > 0
    ) {
      const first = queuedLoadsRef.current.entries().next().value as
        | [string, FileLoadDescriptor]
        | undefined
      if (first === undefined) return
      const [path, descriptor] = first
      queuedLoadsRef.current.delete(path)
      const desired = desiredLoadsRef.current.get(path)
      if (desired === undefined || !sameLoadDescriptor(descriptor, desired)) continue

      const cached = cacheRef.current.get(path)
      if (
        cached !== undefined &&
        sameLoadDescriptor(cached, descriptor) &&
        cached.state.phase !== 'idle' &&
        cached.state.phase !== 'loading'
      ) {
        continue
      }

      activeLoadsRef.current.add(path)
      cacheRef.current.set(path, { ...descriptor, state: { phase: 'loading' } })
      bumpCache()
      void loadReviewContents(descriptor.host, descriptor.root, descriptor.entry)
        .then<FileState>((contents) => ({ phase: 'ready', ...contents }))
        .catch<FileState>((error: unknown) => ({
          phase: 'error',
          message: errorMessage(error),
        }))
        .then((state) => {
          const latest = availableLoadsRef.current.get(path)
          if (latest !== undefined && sameLoadDescriptor(latest, descriptor)) {
            cacheRef.current.set(path, { ...descriptor, state })
            bumpCache()
          }
        })
        .finally(() => {
          activeLoadsRef.current.delete(path)
          const latest = desiredLoadsRef.current.get(path)
          if (latest !== undefined && !sameLoadDescriptor(latest, descriptor)) {
            queuedLoadsRef.current.delete(path)
            queuedLoadsRef.current.set(path, latest)
          }
          pumpLoadsRef.current()
        })
    }
  }, [bumpCache])
  pumpLoadsRef.current = pumpLoads

  useEffect(() => {
    const available = new Map<string, FileLoadDescriptor>()
    for (const entry of entries) {
      available.set(entry.path, { host, root, refreshEpoch, entry })
    }
    availableLoadsRef.current = available

    const requested = desiredReviewEntries(
      entries,
      activePath,
      collapsed,
      viewportRequestedPaths,
    )
    const desired = new Map<string, FileLoadDescriptor>()
    for (const entry of requested) {
      desired.set(entry.path, { host, root, refreshEpoch, entry })
    }
    desiredLoadsRef.current = desired

    for (const [path, cached] of cacheRef.current) {
      const descriptor = available.get(path)
      if (descriptor === undefined || !sameLoadDescriptor(cached, descriptor)) {
        cacheRef.current.delete(path)
      }
    }

    const queued = new Map<string, FileLoadDescriptor>()
    for (const [path, descriptor] of desired) {
      if (activeLoadsRef.current.has(path)) continue
      const cached = cacheRef.current.get(path)
      if (
        cached !== undefined &&
        sameLoadDescriptor(cached, descriptor) &&
        (cached.state.phase === 'ready' || cached.state.phase === 'error')
      ) {
        continue
      }
      queued.set(path, descriptor)
    }
    queuedLoadsRef.current = queued
    pumpLoadsRef.current()
  }, [
    host,
    root,
    entries,
    activePath,
    collapsed,
    viewportRequestedPaths,
    refreshEpoch,
  ])

  useEffect(() => {
    const paths = new Set(entries.map((entry) => entry.path))
    const previouslySeen = seenPathsRef.current
    const enteredLargeReview = largeReview && !wasLargeReviewRef.current
    setCollapsed((previous) => {
      const next = new Set([...previous].filter((path) => paths.has(path)))
      const nextAuto = new Set(
        [...autoCollapsedRef.current].filter((path) => paths.has(path)),
      )
      if (!largeReview) {
        for (const path of nextAuto) next.delete(path)
        nextAuto.clear()
      } else {
        for (const entry of entries.slice(LARGE_REVIEW_EAGER_FILE_COUNT)) {
          if (
            entry.path !== activePath &&
            (enteredLargeReview || !previouslySeen.has(entry.path))
          ) {
            next.add(entry.path)
            nextAuto.add(entry.path)
          }
        }
      }
      autoCollapsedRef.current = nextAuto
      if (
        next.size === previous.size &&
        [...next].every((path) => previous.has(path))
      ) {
        return previous
      }
      return next
    })
    seenPathsRef.current = paths
    wasLargeReviewRef.current = largeReview
    setViewportRequestedPaths((previous) => {
      const next = new Set([...previous].filter((path) => paths.has(path)))
      return next.size === previous.size ? previous : next
    })
  }, [entries, largeReview, activePath])

  useEffect(() => {
    if (collapseEpoch > 0) {
      autoCollapsedRef.current = new Set()
      setCollapsed(new Set(entriesRef.current.map((entry) => entry.path)))
    }
  }, [collapseEpoch])
  useEffect(() => {
    if (expandEpoch > 0) {
      autoCollapsedRef.current = new Set()
      setCollapsed(new Set())
    }
  }, [expandEpoch])
  useEffect(() => {
    if (activePath === null) return
    autoCollapsedRef.current.delete(activePath)
    setCollapsed((previous) => expandedReviewPaths(previous, activePath))
    const frame = window.requestAnimationFrame(() => {
      const escaped = window.CSS?.escape ? window.CSS.escape(activePath) : activePath.replace(/["\\]/g, '\\$&')
      bodyRef.current
        ?.querySelector<HTMLElement>(`[data-review-path="${escaped}"]`)
        ?.scrollIntoView({ block: 'start', behavior: 'auto' })
    })
    return () => window.cancelAnimationFrame(frame)
  }, [activePath, refreshEpoch])

  const onStats = useCallback((summary: ReviewFileSummary | null, path: string) => {
    setStats((previous) => {
      const next = new Map(previous)
      if (summary === null) next.delete(path)
      else next.set(path, summary)
      return next
    })
  }, [])

  const requestViewportLoad = useCallback((path: string) => {
    setViewportRequestedPaths((previous) => {
      if (previous.has(path)) return previous
      const next = new Set(previous)
      next.add(path)
      return next
    })
  }, [])

  const saveFile = useCallback(
    async (
      entry: ReviewEntry,
      state: Extract<FileState, { phase: 'ready' }>,
      contents: string,
      expectedRevision: string | undefined,
    ) => {
      const worktreePath = reviewEntryWorktreePath(entry)
      if (worktreePath === null) throw new Error('this comparison is read-only')
      if (expectedRevision === undefined) {
        throw new Error('reload the file before editing so its revision can be verified')
      }
      const result = await coderWriteFile(
        host,
        joinPath(root, worktreePath),
        contents,
        state.mode,
        expectedRevision,
      )
      if (!result.success) {
        throw new Error(result.error?.message ?? 'save failed')
      }
      const descriptor = availableLoadsRef.current.get(entry.path)
      if (descriptor !== undefined) {
        cacheRef.current.set(entry.path, {
          ...descriptor,
          state: {
            ...state,
            newContents: contents,
            worktreeRevision: result.revision ?? expectedRevision,
          },
        })
        bumpCache()
      }
      onFileSaved(entry.path, contents, result.revision ?? expectedRevision)
    },
    [host, root, bumpCache, onFileSaved],
  )

  const fileStates = useMemo(() => {
    const states = new Map<string, FileState>()
    for (const entry of entries) {
      const descriptor = { host, root, refreshEpoch, entry }
      const cached = cacheRef.current.get(entry.path)
      states.set(
        entry.path,
        cached !== undefined && sameLoadDescriptor(cached, descriptor)
          ? cached.state
          : IDLE_FILE_STATE,
      )
    }
    return states
  }, [host, root, entries, refreshEpoch, cacheVersion])

  const orderedStats = useMemo(() => {
    const readyPaths = new Set<string>()
    const unavailablePaths = new Set<string>()
    for (const [path, state] of fileStates) {
      if (state.phase === 'ready') readyPaths.add(path)
      else if (state.phase === 'error') unavailablePaths.add(path)
    }
    return orderedReviewSummaries(
      entries,
      stats,
      readyPaths,
      unavailablePaths,
    )
  }, [entries, stats, fileStates])
  useEffect(() => onSummaryChange(orderedStats), [orderedStats, onSummaryChange])

  return (
    <div ref={bodyRef} className="shui-review-body">
      {entries.length === 0 ? (
        <div className="shui-main-empty">
          <span className="t-ghost">no files changed in this review</span>
        </div>
      ) : (
        entries.map((entry, index) => (
          <ReviewFile
            key={entry.path}
            entry={entry}
            options={options}
            state={fileStates.get(entry.path) ?? IDLE_FILE_STATE}
            collapsed={collapsed.has(entry.path)}
            active={activePath === entry.path}
            largeReview={largeReview}
            eagerlyRender={index < LARGE_REVIEW_EAGER_FILE_COUNT}
            scrollRoot={bodyRef}
            onRequestLoad={requestViewportLoad}
            onRequestEdit={onRequestEdit}
            onEditDraftChange={onEditDraftChange}
            onEditDirtyChange={onEditDirtyChange}
            onEditSavingChange={onEditSavingChange}
            onSave={saveFile}
            onOpenLine={onOpenLine}
            onActivate={() => onActivate(entry.path)}
            onToggle={() => {
              const isCollapsed = collapsed.has(entry.path)
              // An already-active expanded header is a pure collapse action.
              // Other header clicks activate the file, while Files-tree repeat
              // activations arrive through refreshEpoch and reopen it below.
              if (activePath !== entry.path || isCollapsed) onActivate(entry.path)
              autoCollapsedRef.current.delete(entry.path)
              setCollapsed((previous) => {
                const next = new Set(previous)
                if (next.has(entry.path)) next.delete(entry.path)
                else next.add(entry.path)
                return next
              })
            }}
            onStats={onStats}
          />
        ))
      )}
    </div>
  )
}

function ReviewFile({
  entry,
  options,
  state,
  collapsed,
  active,
  largeReview,
  eagerlyRender,
  scrollRoot,
  onRequestLoad,
  onRequestEdit,
  onEditDraftChange,
  onEditDirtyChange,
  onEditSavingChange,
  onSave,
  onOpenLine,
  onActivate,
  onToggle,
  onStats,
}: {
  entry: ReviewEntry
  options: ReviewOptions
  state: FileState
  collapsed: boolean
  active: boolean
  largeReview: boolean
  eagerlyRender: boolean
  scrollRoot: React.RefObject<HTMLDivElement | null>
  onRequestLoad: (path: string) => void
  onRequestEdit: (path: string) => boolean
  onEditDraftChange: (path: string, draft: ReviewEditDraft | null) => void
  onEditDirtyChange: (path: string, dirty: boolean) => void
  onEditSavingChange: (path: string, saving: boolean) => void
  onSave: (
    entry: ReviewEntry,
    state: Extract<FileState, { phase: 'ready' }>,
    contents: string,
    expectedRevision: string | undefined,
  ) => Promise<void>
  onOpenLine?: (path: string, line: number) => void
  onActivate: () => void
  onToggle: () => void
  onStats: (summary: ReviewFileSummary | null, path: string) => void
}) {
  const sectionRef = useRef<HTMLElement>(null)
  const [renderBody, setRenderBody] = useState(
    !largeReview || eagerlyRender || active,
  )
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [editBaseContents, setEditBaseContents] = useState('')
  const [editRevision, setEditRevision] = useState<string | undefined>()
  const [saving, setSaving] = useState(false)
  const savingRef = useRef(false)
  const [editorState, setEditorState] = useState<FileDiffEditState>('loading')
  const [saveError, setSaveError] = useState<string | null>(null)

  useEffect(() => {
    if (active) {
      setRenderBody(true)
      onRequestLoad(entry.path)
      return
    }
    if (!largeReview || eagerlyRender) {
      setRenderBody(true)
      return
    }
    if (collapsed) {
      setRenderBody(false)
      return
    }
    if (renderBody) return
    const section = sectionRef.current
    if (section === null || typeof IntersectionObserver === 'undefined') {
      setRenderBody(true)
      onRequestLoad(entry.path)
      return
    }
    const observer = new IntersectionObserver(
      (observations) => {
        if (!observations.some((observation) => observation.isIntersecting)) return
        setRenderBody(true)
        onRequestLoad(entry.path)
        observer.disconnect()
      },
      {
        root: scrollRoot.current,
        rootMargin: '800px 0px',
      },
    )
    observer.observe(section)
    return () => observer.disconnect()
  }, [
    largeReview,
    eagerlyRender,
    active,
    collapsed,
    renderBody,
    scrollRoot,
    onRequestLoad,
    entry.path,
  ])

  const totals = useMemo(
    () =>
      state.phase === 'ready'
        ? totalsFor(state.oldContents, state.newContents, options.hideWhitespace)
        : null,
    [state, options.hideWhitespace],
  )
  const openable =
    onOpenLine !== undefined &&
    reviewEntryWorktreePath(entry) !== null &&
    state.phase === 'ready' &&
    state.image === undefined &&
    !state.imageUnavailable
  const lineOps = useMemo(
    () => (openable && state.phase === 'ready' ? diffLines(state.oldContents, state.newContents) : []),
    [openable, state],
  )
  const openAtLine = (line: number) => {
    const path = reviewEntryWorktreePath(entry)
    if (path !== null && onOpenLine) onOpenLine(path, line)
  }
  const openFromGutter = (event: React.MouseEvent<HTMLElement>) => {
    if (!openable || editing) return
    const target = gutterLineFromPath(event.nativeEvent.composedPath())
    if (target === null) return
    event.preventDefault()
    openAtLine(resolveEditorLine(lineOps, target))
  }
  useEffect(() => {
    onStats(
      totals === null || state.phase !== 'ready'
        ? null
        : {
            path: entry.path,
            state: 'ready',
            ...totals,
            oldContents: state.oldContents,
            newContents: state.newContents,
          },
      entry.path,
    )
  }, [totals, entry.path, onStats])

  const editable =
    reviewEntryWorktreePath(entry) !== null &&
    state.phase === 'ready' &&
    state.worktreeRevision !== undefined &&
    state.image === undefined
  const dirty = editing && draft !== editBaseContents
  const changedOnDisk =
    editing &&
    editRevision !== undefined &&
    state.phase === 'ready' &&
    state.worktreeRevision !== undefined &&
    state.worktreeRevision !== editRevision

  useEffect(() => {
    reportActiveReviewDirty(editing, entry.path, dirty, onEditDirtyChange)
  }, [editing, entry.path, dirty, onEditDirtyChange])

  useEffect(() => {
    if (!editing) return
    onEditDraftChange(entry.path, {
      savedContent: editBaseContents,
      draft,
      revision: editRevision,
      mode: state.phase === 'ready' ? state.mode : undefined,
    })
  }, [
    editing,
    entry.path,
    editBaseContents,
    draft,
    editRevision,
    state,
    onEditDraftChange,
  ])

  const beginEditing = useCallback(() => {
    if (!editable || state.phase !== 'ready') return
    if (!onRequestEdit(entry.path)) return
    onRequestLoad(entry.path)
    if (collapsed) onToggle()
    else onActivate()
    setDraft(state.newContents)
    setEditBaseContents(state.newContents)
    setEditRevision(state.worktreeRevision)
    savingRef.current = false
    setEditorState('loading')
    setSaveError(null)
    setEditing(true)
  }, [
    editable,
    state,
    entry.path,
    collapsed,
    onRequestEdit,
    onRequestLoad,
    onToggle,
    onActivate,
  ])

  const cancelEditing = useCallback(() => {
    if (savingRef.current) return
    if (dirty && !window.confirm(`discard unsaved changes to ${entry.path}?`)) return
    setDraft(editBaseContents)
    setSaveError(null)
    setEditing(false)
    onEditDraftChange(entry.path, null)
    onEditDirtyChange(entry.path, false)
  }, [dirty, entry.path, editBaseContents, onEditDraftChange, onEditDirtyChange])

  const reloadEditing = useCallback(() => {
    if (savingRef.current || state.phase !== 'ready') return
    if (dirty && !window.confirm(`reload ${entry.path} and discard your draft?`)) return
    setDraft(state.newContents)
    setEditBaseContents(state.newContents)
    setEditRevision(state.worktreeRevision)
    setSaveError(null)
    onEditDirtyChange(entry.path, false)
  }, [state, dirty, entry.path, onEditDirtyChange])

  const saveEditing = useCallback(() => {
    if (
      !editing ||
      !dirty ||
      savingRef.current ||
      editorState !== 'ready' ||
      state.phase !== 'ready'
    ) {
      return
    }
    const submittedDraft = draft
    let saved = false
    savingRef.current = true
    setSaving(true)
    setSaveError(null)
    void withReviewSavePending(entry.path, onEditSavingChange, async () => {
      try {
        await onSave(entry, state, submittedDraft, editRevision)
        saved = true
        setEditBaseContents(submittedDraft)
        setEditRevision(undefined)
        setEditing(false)
        onEditDraftChange(entry.path, null)
        onEditDirtyChange(entry.path, false)
      } catch (error: unknown) {
        setSaveError(errorMessage(error))
      }
    })
      .finally(() => {
        if (!saved) savingRef.current = false
        setSaving(false)
      })
  }, [
    editing,
    dirty,
    editorState,
    state,
    onSave,
    entry,
    draft,
    editRevision,
    onEditDraftChange,
    onEditDirtyChange,
    onEditSavingChange,
  ])

  const editStatus = reviewEditStatus(editing, editorState, saving)

  const rich =
    renderBody && state.phase === 'ready' ? richPreviewFor(entry.path, state) : null
  return (
    <section
      ref={sectionRef}
      className={`shui-review-file${active ? ' active' : ''}`}
      data-review-path={entry.path}
      aria-busy={saving}
      onClick={openFromGutter}
      onKeyDownCapture={(event) => {
        if (
          editing &&
          (event.metaKey || event.ctrlKey) &&
          event.key.toLowerCase() === 's'
        ) {
          event.preventDefault()
          saveEditing()
        }
      }}
    >
      <div className="shui-review-file-head">
        <button
          type="button"
          className="shui-review-file-toggle"
          onClick={onToggle}
          aria-expanded={!collapsed}
        >
          <ChevronRight aria-hidden className={collapsed ? 'chevron' : 'chevron open'} />
          <FileTypeIcon path={entry.path} className="file-icon" />
          <span className="path" title={entry.path}>
            {entry.change.from ? `${entry.change.from} → ${entry.path}` : entry.path}
          </span>
        </button>
        {totals !== null ? (
          <span className="shui-diff-stats">
            <span className="add">+{totals.add}</span>
            <span className="del">−{totals.del}</span>
          </span>
        ) : null}
        {dirty ? <span className="shui-dirty" title="unsaved diff edit" /> : null}
        {openable && !editing ? (
          <button
            type="button"
            className="shui-review-file-action"
            onClick={() => openAtLine(firstChangedLine(lineOps))}
            aria-label={`open ${entry.path} in the editor`}
            title="open in editor at the first change (or click a line number)"
          >
            <FileSymlink aria-hidden />
          </button>
        ) : null}
        {editable && state.phase === 'ready' ? (
          editing ? (
            <>
              <button
                type="button"
                className="shui-review-file-action"
                onClick={cancelEditing}
                disabled={saving}
                aria-label={`cancel editing ${entry.path}`}
                title="cancel edit"
              >
                <X aria-hidden />
              </button>
              <button
                type="button"
                className="shui-review-file-action primary"
                onClick={saveEditing}
                disabled={!dirty || saving || editorState !== 'ready'}
                aria-label={`save ${entry.path}`}
                title="save (⌘S)"
              >
                <Save aria-hidden />
              </button>
            </>
          ) : (
            <button
              type="button"
              className="shui-review-file-action"
              onClick={beginEditing}
              aria-label={`edit working file ${entry.path}`}
              title="edit working file"
            >
              <Pencil aria-hidden />
            </button>
          )
        ) : null}
      </div>
      {changedOnDisk ? (
        <div className="shui-review-message warn">
          <span>file changed on disk</span>
          <button
            type="button"
            className="shui-review-inline-action"
            onClick={reloadEditing}
            disabled={saving}
          >
            reload latest
          </button>
        </div>
      ) : saveError ? (
        <div className="shui-review-message warn">{saveError}</div>
      ) : editStatus ? (
        <div
          className={`shui-review-message${editStatus.role === 'alert' ? ' warn' : ''}`}
          role={editStatus.role}
        >
          {editStatus.message}
        </div>
      ) : null}
      {!collapsed &&
      renderBody &&
      state.phase === 'ready' &&
      state.baselineSource === 'committed' ? (
        <div className="shui-review-message" role="status">
          compared against the last commit — this turn's earlier content was
          not captured, so edits made before it are included
        </div>
      ) : null}
      {collapsed ? null : !renderBody ? (
        <div className="shui-review-message">scroll to load diff…</div>
      ) : state.phase === 'idle' || state.phase === 'loading' ? (
        <div className="shui-review-message">loading diff…</div>
      ) : state.phase === 'error' ? (
        <div className="shui-review-message warn">{state.message}</div>
      ) : !editing && options.richPreview && rich !== null ? (
        rich
      ) : !editing && state.imageUnavailable ? (
        <div className="shui-review-message">
          binary image; preview is available for working-tree files only
        </div>
      ) : !editing && state.image !== undefined ? (
        <div className="shui-review-message">
          {state.image === null ? 'image deleted' : 'binary image; enable rich preview to view it'}
        </div>
      ) : !editing && totals?.add === 0 && totals.del === 0 ? (
        <div className="shui-review-message">no line changes</div>
      ) : (
        <FileDiff
          key={options.diffStyle}
          oldFile={{ name: entry.change.from ?? entry.path, contents: state.oldContents }}
          newFile={{ name: entry.path, contents: editing ? draft : state.newContents }}
          diffStyle={options.diffStyle}
          overflow={options.wordWrap ? 'wrap' : 'scroll'}
          lineDiffType={options.wordDiffs ? 'word-alt' : 'none'}
          ignoreWhitespace={options.hideWhitespace}
          expandUnchanged={options.expandUnchanged}
          disableFileHeader
          edit={editing && !saving}
          onChange={
            editing && !saving
              ? (next) => updateReviewDraft(savingRef, next, setDraft)
              : undefined
          }
          onEditStateChange={editing && !saving ? setEditorState : undefined}
          className="shui-review-diff"
        />
      )}
    </section>
  )
}

function richPreviewFor(
  path: string,
  state: Extract<FileState, { phase: 'ready' }>,
): React.ReactNode | null {
  if (state.image !== undefined) {
    return state.image === null ? null : (
      <img className="shui-rich-preview-image" src={state.image} alt={`preview ${path}`} />
    )
  }
  return richPreviewNode(path, state.newContents)
}

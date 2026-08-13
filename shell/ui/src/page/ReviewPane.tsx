import { FileDiff, type Host } from '@iii-dev/console-ui'
import { ChevronDown, ChevronRight, FileCode2 } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import { coderReadFile, joinPath, type ReadFileResponse } from './coder'
import { diffLines, diffTotals } from './diff'
import { gitReadSource, gitShowHead } from './git'
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
  onSummaryChange: (files: readonly ReviewFileSummary[]) => void
}

type FileState =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'error'; message: string }
  | { phase: 'ready'; oldContents: string; newContents: string }

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

async function loadReviewContents(
  host: Host,
  root: string,
  entry: ReviewEntry,
): Promise<{ oldContents: string; newContents: string }> {
  if (entry.before !== undefined && entry.after !== undefined) {
    const [oldContents, newContents] = await Promise.all([
      gitReadSource(host, root, entry.before),
      gitReadSource(host, root, entry.after),
    ])
    return { oldContents, newContents }
  }
  if (entry.baseline === null) {
    throw new Error('earlier content was not captured for this turn')
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
  const newSide =
    change.status === 'deleted'
      ? Promise.resolve('')
      : coderReadFile(host, joinPath(root, change.path)).then((out) =>
          exactCoderText(out, change.path),
        )
  const [oldContents, newContents] = await Promise.all([oldSide, newSide])
  return { oldContents, newContents }
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
  onSummaryChange,
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
      setCollapsed(new Set(entries.map((entry) => entry.path)))
    }
  }, [collapseEpoch, entries])
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
  onToggle: () => void
  onStats: (summary: ReviewFileSummary | null, path: string) => void
}) {
  const sectionRef = useRef<HTMLElement>(null)
  const [renderBody, setRenderBody] = useState(
    !largeReview || eagerlyRender || active,
  )

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

  const rich =
    renderBody && state.phase === 'ready'
      ? richPreviewFor(entry.path, state.newContents)
      : null
  return (
    <section
      ref={sectionRef}
      className={`shui-review-file${active ? ' active' : ''}`}
      data-review-path={entry.path}
    >
      <button type="button" className="shui-review-file-head" onClick={onToggle}>
        {collapsed ? <ChevronRight aria-hidden /> : <ChevronDown aria-hidden />}
        <FileCode2 aria-hidden className="file-icon" />
        <span className="path" title={entry.path}>
          {entry.change.from ? `${entry.change.from} → ${entry.path}` : entry.path}
        </span>
        {totals !== null ? (
          <span className="shui-diff-stats">
            <span className="add">+{totals.add}</span>
            <span className="del">−{totals.del}</span>
          </span>
        ) : null}
      </button>
      {collapsed ? null : !renderBody ? (
        <div className="shui-review-message">scroll to load diff…</div>
      ) : state.phase === 'idle' || state.phase === 'loading' ? (
        <div className="shui-review-message">loading diff…</div>
      ) : state.phase === 'error' ? (
        <div className="shui-review-message warn">{state.message}</div>
      ) : options.richPreview && rich !== null ? (
        rich
      ) : totals?.add === 0 && totals.del === 0 ? (
        <div className="shui-review-message">no line changes</div>
      ) : (
        <FileDiff
          oldFile={{ name: entry.change.from ?? entry.path, contents: state.oldContents }}
          newFile={{ name: entry.path, contents: state.newContents }}
          diffStyle={options.diffStyle}
          overflow={options.wordWrap ? 'wrap' : 'scroll'}
          lineDiffType={options.wordDiffs ? 'word-alt' : 'none'}
          ignoreWhitespace={options.hideWhitespace}
          expandUnchanged={options.expandUnchanged}
          disableFileHeader
          className="shui-review-diff"
        />
      )}
    </section>
  )
}

function richPreviewFor(path: string, contents: string): React.ReactNode | null {
  const lower = path.toLowerCase()
  if (lower.endsWith('.html') || lower.endsWith('.htm')) {
    return <iframe className="shui-rich-preview" title={`preview ${path}`} sandbox="" srcDoc={contents} />
  }
  if (lower.endsWith('.svg')) {
    const src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(contents)}`
    return <img className="shui-rich-preview-image" src={src} alt={`preview ${path}`} />
  }
  if (lower.endsWith('.md') || lower.endsWith('.markdown')) {
    return <MarkdownPreview contents={contents} />
  }
  return null
}

function MarkdownPreview({ contents }: { contents: string }) {
  return (
    <article className="shui-markdown-preview">
      {contents.split('\n').map((line, index) => {
        const heading = /^(#{1,4})\s+(.*)$/.exec(line)
        if (heading) {
          const level = heading[1].length
          const text = heading[2]
          if (level === 1) return <h1 key={index}>{text}</h1>
          if (level === 2) return <h2 key={index}>{text}</h2>
          if (level === 3) return <h3 key={index}>{text}</h3>
          return <h4 key={index}>{text}</h4>
        }
        if (/^[-*]\s+/.test(line)) return <li key={index}>{line.slice(2)}</li>
        if (line.trim() === '') return <br key={index} />
        return <p key={index}>{line}</p>
      })}
    </article>
  )
}

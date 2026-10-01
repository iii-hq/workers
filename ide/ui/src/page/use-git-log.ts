/* The Git window's Log as React state: the refs, the commits a filter and
   a branch pick leave, and their graph, read only while the log shows.

   The refs are re-read (a few tens of milliseconds) when anything might
   have moved them: the root, the caller's refresh key, window focus, the
   tab coming back, a new filter or branch. When their listing is unchanged
   nothing else runs (a new filter's log still starts over, once); when it
   changed, the log starts over from the new tips, reading as many rows as
   were loaded so the view keeps its place. A read that a newer one
   superseded is dropped: `shell::exec` cannot be aborted. */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { emptyGraphState, type GraphRow, type GraphState, layoutPage } from './commit-graph'
import {
  type CommitDetails,
  type CommitFile,
  findCommit,
  type LogCommit,
  type LogFilter,
  labelsBySha,
  logTips,
  type RefsSnapshot,
  readCommitDetails,
  readContainingBranches,
  readLogPage,
  readRefs,
  readWorkingDiff,
  showsGraph,
} from './git-log-window'

const PAGE = 1000
const DETAILS_DEBOUNCE_MS = 150
const BRANCHES_DELAY_MS = 300
const DETAILS_CACHE = 200

export interface GitLogState {
  /** Null before the first read, and outside a repository (`notRepo`). */
  snapshot: RefsSnapshot | null
  notRepo: boolean
  commits: LogCommit[]
  /** The graph, one row per commit; null when the filter hides it. */
  graph: GraphRow[] | null
  /** The history ran out: every commit is loaded. */
  done: boolean
  loading: boolean
  error: string | null
  /** Reads the next page, if there is one and none is on its way. */
  loadMore(): void
  /** Reads the refs again, and the log if they moved. */
  refresh(): void
}

/** The ref name a lane opened at `sha` takes its colour from: a local
    branch there, else a remote one. */
function laneKeys(snapshot: RefsSnapshot): (sha: string) => string | null {
  const labels = labelsBySha(snapshot)
  return (sha) => {
    const refs = labels.get(sha) ?? []
    return (refs.find((ref) => ref.kind === 'local') ?? refs.find((ref) => ref.kind === 'remote'))?.name ?? null
  }
}

/** `tipRef` is a ref's full name (or 'HEAD') the log is limited to; null
    for everything. `refreshKey` re-reads the refs whenever it changes. The
    filter applies at once: typed text settles in its field first. */
export function useGitLog(
  host: Host,
  root: string | null,
  refreshKey: unknown,
  active: boolean,
  filter: LogFilter,
  tipRef: string | null,
): GitLogState {
  const [snapshot, setSnapshot] = useState<RefsSnapshot | null>(null)
  const [notRepo, setNotRepo] = useState(false)
  const [commits, setCommits] = useState<LogCommit[]>([])
  const [graph, setGraph] = useState<GraphRow[] | null>(null)
  const [done, setDone] = useState(false)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [refreshes, setRefreshes] = useState(0)

  // A filter picked again as it was reads nothing.
  const filterKey = JSON.stringify(filter)

  // Everything a page read needs, read at call time: a stale closure must
  // not append to a newer generation.
  const live = useRef({
    generation: 0,
    snapshot: null as RefsSnapshot | null,
    /** The snapshot's lane colours, worked out once rather than per page. */
    laneKey: (() => null) as (sha: string) => string | null,
    commits: [] as LogCommit[],
    graphState: emptyGraphState as GraphState,
    graph: [] as GraphRow[],
    nextSkip: 0,
    done: false,
    pending: false,
    /** A read of this generation failed: no more pages until a restart. */
    failed: false,
  })

  const load = useCallback(
    async (generation: number, skip: number, limit: number) => {
      const state = live.current
      const current = state.snapshot
      if (root === null || current === null) return
      const parsed = JSON.parse(filterKey) as LogFilter
      state.pending = true
      setLoading(true)
      try {
        const tips = parsed.upTo ? [parsed.upTo] : logTips(current, tipRef)
        const [page, found] = await Promise.all([
          readLogPage(host, root, tips, parsed, skip, limit),
          skip === 0 && parsed.text ? findCommit(host, root, parsed.text.trim()) : Promise.resolve(null),
        ])
        if (state.generation !== generation || state.nextSkip !== skip) return
        let added = page.commits
        // A hash typed into the filter finds its commit ahead of the matches.
        if (found !== null && !added.some((commit) => commit.sha === found.sha)) added = [found, ...added]
        state.commits = skip === 0 ? added : [...state.commits, ...added]
        state.nextSkip = skip + page.commits.length
        state.done = page.done
        if (showsGraph(parsed)) {
          const laid = layoutPage(skip === 0 ? emptyGraphState : state.graphState, added, state.laneKey)
          state.graphState = laid.state
          state.graph = skip === 0 ? laid.rows : [...state.graph, ...laid.rows]
          setGraph(state.graph)
        } else {
          setGraph(null)
        }
        setCommits(state.commits)
        setDone(state.done)
        setError(null)
      } catch (err: unknown) {
        if (state.generation !== generation) return
        state.failed = true
        setError(err instanceof Error ? err.message : String(err))
        // The first page failed: what shows would be another filter's rows.
        if (skip === 0) {
          state.commits = []
          state.graph = []
          setCommits([])
          setGraph(null)
        }
      } finally {
        if (state.generation === generation) {
          state.pending = false
          setLoading(false)
        }
      }
    },
    [host, root, filterKey, tipRef],
  )

  // A new generation: the log starts over from the tips.
  const restart = useCallback(
    (keep: number) => {
      const state = live.current
      state.generation += 1
      state.commits = []
      state.graphState = emptyGraphState
      state.graph = []
      state.nextSkip = 0
      state.done = false
      state.pending = false
      state.failed = false
      void load(state.generation, 0, Math.max(PAGE, keep))
    },
    [load],
  )
  // The refs, whenever anything may have moved them.
  const refSeq = useRef(0)
  // The log starts over even when the refs did not move: Refresh keeps
  // the rows loaded, a new filter or branch reads from its first page.
  const force = useRef<'refresh' | 'filter' | null>(null)
  const readRefsNow = useCallback(() => {
    if (!active || root === null) return
    const seq = ++refSeq.current
    readRefs(host, root).then(
      (next) => {
        if (seq !== refSeq.current) return
        const state = live.current
        if (next === null) {
          // The read of a page still in flight belongs to the dropped
          // generation and will not clear its own loading.
          state.snapshot = null
          state.generation += 1
          state.pending = false
          state.commits = []
          setLoading(false)
          setError(null)
          setSnapshot(null)
          setNotRepo(true)
          setCommits([])
          setGraph(null)
          setDone(true)
          return
        }
        setNotRepo(false)
        const moved = state.snapshot?.signature !== next.signature
        if (!moved && force.current === null) return
        const loaded = force.current === 'filter' ? 0 : state.commits.length
        // Unmoved refs keep their snapshot for a new filter: the branch
        // tree and every chip are built from it.
        if (moved || force.current === 'refresh') {
          state.snapshot = next
          state.laneKey = laneKeys(next)
          setSnapshot(next)
        }
        force.current = null
        restart(loaded)
      },
      (err: unknown) => {
        if (seq !== refSeq.current) return
        setError(err instanceof Error ? err.message : String(err))
        // A new filter or branch still applies, from the refs already read.
        if (force.current === 'filter' && live.current.snapshot !== null) {
          force.current = null
          restart(0)
        }
      },
    )
  }, [host, root, active, restart])

  useEffect(() => {
    readRefsNow()
  }, [readRefsNow, refreshKey, refreshes])

  // The filter or the branch changed: a new `restart`, so a new
  // `readRefsNow` reads the refs above (a commit or a fetch in a terminal
  // moves them unseen), and the log starts over once they land, whether
  // or not they moved. A new root forgets the previous repository instead;
  // its own refs read restarts it.
  const lastRoot = useRef(root)
  const firstRun = useRef(true)
  useEffect(() => {
    const state = live.current
    if (lastRoot.current !== root) {
      lastRoot.current = root
      state.snapshot = null
      state.generation += 1
      state.pending = false
      state.commits = []
      setLoading(false)
      setError(null)
      setSnapshot(null)
      setCommits([])
      setGraph(null)
      setDone(false)
      return
    }
    if (firstRun.current) {
      firstRun.current = false
      return
    }
    if (state.snapshot !== null) force.current = 'filter'
  }, [restart, root])

  // Coming back to the window: a commit or a fetch in a terminal moves refs
  // the file watcher does not see (it leaves `.git` out).
  useEffect(() => {
    if (!active) return
    // Coming back fires both focus and visibilitychange: one read.
    let last = 0
    const once = () => {
      if (Date.now() - last < 1000) return
      last = Date.now()
      readRefsNow()
    }
    const onFocus = once
    const onVisible = () => {
      if (document.visibilityState === 'visible') once()
    }
    window.addEventListener('focus', onFocus)
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      window.removeEventListener('focus', onFocus)
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [active, readRefsNow])

  const loadMore = useCallback(() => {
    const state = live.current
    if (state.done || state.pending || state.failed || state.snapshot === null) return
    // A new filter waits for its refs read: its first page comes from there,
    // not from the old filter's offset.
    if (force.current === 'filter') return
    void load(state.generation, state.nextSkip, PAGE)
  }, [load])

  const refresh = useCallback(() => {
    force.current = 'refresh'
    setRefreshes((value) => value + 1)
  }, [])

  // One object while nothing in it changed: the Log's panes are memoized.
  return useMemo(
    () => ({ snapshot, notRepo, commits, graph, done, loading, error, loadMore, refresh }),
    [snapshot, notRepo, commits, graph, done, loading, error, loadMore, refresh],
  )
}

export interface CommitDetailsState {
  details: CommitDetails | null
  loading: boolean
  error: string | null
  /** The branches that have the commit, read once the selection rests. */
  branches: Branches | null
}

type Branches = { names: string[]; total: number; partial: boolean }

/** A read of one commit's details as it landed: the commit, or why not. */
interface DetailsRead {
  key: string
  details: CommitDetails | null
  error: string | null
}

// Kept across mounts, so a Git window hidden and shown again has the
// commits it read; the keys name the root, and all else a read depends on.
const detailsCache = new Map<string, CommitDetails>()
const branchesCache = new Map<string, Branches>()

function remember<T>(cache: Map<string, T>, key: string, value: T) {
  cache.set(key, value)
  // ponytail: drops the oldest entry; an LRU if revisits matter.
  if (cache.size > DETAILS_CACHE) cache.delete(cache.keys().next().value as string)
}

/** What the details pane shows for `key`: the cached commit, else what the
    last read left if it was for this key; until then it is loading. */
export function detailsFor(
  key: string | null,
  cached: CommitDetails | undefined,
  read: DetailsRead | null,
): Omit<CommitDetailsState, 'branches'> {
  const own = read !== null && read.key === key ? read : null
  const details = key === null ? null : (cached ?? own?.details ?? null)
  const error = details === null ? (own?.error ?? null) : null
  return { details, loading: key !== null && details === null && error === null, error }
}

/** The selected commit's details, read once the selection settles, and
    kept for the last few hundred commits looked at. Each read lands with
    the key it was for and what shows is worked out from the current key,
    so a new selection takes a single commit. */
export function useCommitDetails(
  host: Host,
  root: string | null,
  snapshot: RefsSnapshot | null,
  sha: string | null,
): CommitDetailsState {
  const prefix = snapshot?.prefix ?? ''
  const signature = snapshot?.signature ?? ''
  // Each file's folder-relative path depends on the folder: wait for it.
  const key = root === null || sha === null || snapshot === null ? null : `${root}\0${prefix}\0${sha}`
  const branchesKey = root === null || sha === null ? null : `${root}\0${sha}\0${signature}`
  const [read, setRead] = useState<DetailsRead | null>(null)
  const [branchesRead, setBranchesRead] = useState<{ key: string; branches: Branches } | null>(null)

  useEffect(() => {
    if (key === null || root === null || sha === null || detailsCache.has(key)) return
    let stale = false
    const timer = setTimeout(() => {
      readCommitDetails(host, root, prefix, sha).then(
        (details) => {
          if (stale) return
          remember(detailsCache, key, details)
          setRead({ key, details, error: null })
        },
        (err: unknown) => {
          if (!stale) setRead({ key, details: null, error: err instanceof Error ? err.message : String(err) })
        },
      )
    }, DETAILS_DEBOUNCE_MS)
    return () => {
      stale = true
      clearTimeout(timer)
    }
  }, [host, key, root, prefix, sha])

  // Which branches have it: a walk over every ref, so only once the
  // selection rests, and again only when the refs moved.
  useEffect(() => {
    if (branchesKey === null || root === null || sha === null || branchesCache.has(branchesKey)) return
    let stale = false
    const timer = setTimeout(() => {
      readContainingBranches(host, root, sha).then(
        (branches) => {
          if (stale) return
          remember(branchesCache, branchesKey, branches)
          setBranchesRead({ key: branchesKey, branches })
        },
        () => {},
      )
    }, BRANCHES_DELAY_MS)
    return () => {
      stale = true
      clearTimeout(timer)
    }
  }, [host, branchesKey, root, sha])

  const cached = key === null ? undefined : detailsCache.get(key)
  const cachedBranches = branchesKey === null ? undefined : branchesCache.get(branchesKey)
  // A cache hit is kept as this pane's own read: another pane's reads may
  // evict it from the shared cache, and nothing here would read it again.
  // Set while rendering, so React renders again before it commits.
  if (key !== null && cached !== undefined && read?.details !== cached) setRead({ key, details: cached, error: null })
  if (branchesKey !== null && cachedBranches !== undefined && branchesRead?.branches !== cachedBranches) {
    setBranchesRead({ key: branchesKey, branches: cachedBranches })
  }
  const { details, loading, error } = detailsFor(key, cached, read)
  const branches =
    branchesKey === null ? null : (cachedBranches ?? (branchesRead?.key === branchesKey ? branchesRead.branches : null))
  return useMemo(() => ({ details, loading, error, branches }), [details, loading, error, branches])
}

export interface WorkingDiffState {
  files: CommitFile[] | null
  truncated: boolean
  error: string | null
}

/** The files `ref` and the working tree disagree on, read when `ref` is
    set and again when `refreshKey` changes. */
export function useWorkingDiff(
  host: Host,
  root: string | null,
  prefix: string | null,
  ref: string | null,
  refreshKey: unknown,
): WorkingDiffState {
  const [state, setState] = useState<WorkingDiffState & { for: string | null }>({
    for: null,
    files: null,
    truncated: false,
    error: null,
  })
  const key = root === null || prefix === null || ref === null ? null : `${root}\0${prefix}\0${ref}`
  // The key names root, prefix and ref; refreshKey re-reads.
  useEffect(() => {
    if (key === null || root === null || prefix === null || ref === null) return
    let live = true
    readWorkingDiff(host, root, prefix, ref).then(
      (read) => {
        if (live) setState({ for: key, ...read, error: null })
      },
      (err: unknown) => {
        if (live)
          setState({ for: key, files: [], truncated: false, error: err instanceof Error ? err.message : String(err) })
      },
    )
    return () => {
      live = false
    }
  }, [host, key, refreshKey])
  return state.for === key ? state : { files: null, truncated: false, error: null }
}

/* The Git window's Log as React state: the refs, the commits a filter and
   a branch pick leave, and their graph, read only while the log shows.

   The refs are re-read (a few tens of milliseconds) when anything might
   have moved them: the root, the caller's refresh key, window focus, the
   tab coming back. When their listing is unchanged nothing else runs;
   when it changed, the log starts over from the new tips, reading as many
   rows as were loaded so the view keeps its place. A read that a newer one
   superseded is dropped: `shell::exec` cannot be aborted. */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
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
const TEXT_DEBOUNCE_MS = 300
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
    for everything. `refreshKey` re-reads the refs whenever it changes. */
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

  // The filter as applied: typed text settles for a moment first.
  const filterKey = JSON.stringify(filter)
  const [applied, setApplied] = useState(filterKey)
  const textRef = useRef(filter.text)
  useEffect(() => {
    const typed = filter.text !== textRef.current
    textRef.current = filter.text
    const timer = setTimeout(() => setApplied(filterKey), typed ? TEXT_DEBOUNCE_MS : 0)
    return () => clearTimeout(timer)
  }, [filterKey, filter.text])

  // Everything a page read needs, read at call time: a stale closure must
  // not append to a newer generation.
  const live = useRef({
    generation: 0,
    snapshot: null as RefsSnapshot | null,
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
      const parsed = JSON.parse(applied) as LogFilter
      state.pending = true
      setLoading(true)
      try {
        const tips = logTips(current, tipRef)
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
          const laid = layoutPage(skip === 0 ? emptyGraphState : state.graphState, added, laneKeys(current))
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
    [host, root, applied, tipRef],
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
  // Refresh reads the log again even when the refs did not move.
  const force = useRef(false)
  const readRefsNow = useCallback(() => {
    if (!active || root === null) return
    const seq = ++refSeq.current
    readRefs(host, root).then(
      (next) => {
        if (seq !== refSeq.current) return
        const state = live.current
        if (next === null) {
          state.snapshot = null
          state.generation += 1
          setSnapshot(null)
          setNotRepo(true)
          setCommits([])
          setGraph(null)
          setDone(true)
          return
        }
        setNotRepo(false)
        if (state.snapshot?.signature === next.signature && !force.current) return
        force.current = false
        const loaded = state.commits.length
        state.snapshot = next
        setSnapshot(next)
        restart(loaded)
      },
      (err: unknown) => {
        if (seq === refSeq.current) setError(err instanceof Error ? err.message : String(err))
      },
    )
  }, [host, root, active, restart])

  useEffect(() => {
    readRefsNow()
  }, [readRefsNow, refreshKey, refreshes])

  // The filter or the branch changed: same refs, a new log. A new root
  // forgets the previous repository instead; its own refs read restarts it.
  const lastRoot = useRef(root)
  const firstRun = useRef(true)
  useEffect(() => {
    const state = live.current
    if (lastRoot.current !== root) {
      lastRoot.current = root
      state.snapshot = null
      state.generation += 1
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
    if (state.snapshot !== null) restart(0)
  }, [restart, root])

  // Coming back to the window: a commit or a fetch in a terminal moves refs
  // the file watcher does not see (it leaves `.git` out).
  useEffect(() => {
    if (!active) return
    const onFocus = () => readRefsNow()
    const onVisible = () => {
      if (document.visibilityState === 'visible') readRefsNow()
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
    void load(state.generation, state.nextSkip, PAGE)
  }, [load])

  return {
    snapshot,
    notRepo,
    commits,
    graph,
    done,
    loading,
    error,
    loadMore,
    refresh: () => {
      force.current = true
      setRefreshes((value) => value + 1)
    },
  }
}

export interface CommitDetailsState {
  details: CommitDetails | null
  loading: boolean
  error: string | null
  /** The branches that have the commit, read once the selection rests. */
  branches: { names: string[]; total: number; partial: boolean } | null
}

/** The selected commit's details, read once the selection settles, and
    kept for the last few hundred commits looked at. */
export function useCommitDetails(
  host: Host,
  root: string | null,
  snapshot: RefsSnapshot | null,
  sha: string | null,
): CommitDetailsState {
  const cache = useRef(new Map<string, CommitDetails>())
  const branchCache = useRef(new Map<string, { names: string[]; total: number; partial: boolean }>())
  const [details, setDetails] = useState<CommitDetails | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [branches, setBranches] = useState<CommitDetailsState['branches']>(null)
  const prefix = snapshot?.prefix ?? ''
  const signature = snapshot?.signature ?? ''
  // Each file's folder-relative path depends on the folder: wait for it.
  const ready = snapshot !== null

  // Another repository's commits are not this one's.
  const cacheRoot = useRef(root)
  if (cacheRoot.current !== root) {
    cacheRoot.current = root
    cache.current.clear()
    branchCache.current.clear()
  }

  useEffect(() => {
    if (root === null || sha === null || !ready) {
      setDetails(null)
      setBranches(null)
      setError(null)
      return
    }
    const key = `${prefix}\0${sha}`
    const cached = cache.current.get(key)
    setDetails(cached ?? null)
    setError(null)
    let stale = false
    const timer = cached
      ? undefined
      : setTimeout(() => {
          setLoading(true)
          readCommitDetails(host, root, prefix, sha).then(
            (read) => {
              if (stale) return
              cache.current.set(key, read)
              // ponytail: drops the oldest entry; an LRU if revisits matter.
              if (cache.current.size > DETAILS_CACHE) cache.current.delete(cache.current.keys().next().value as string)
              setDetails(read)
              setLoading(false)
            },
            (err: unknown) => {
              if (stale) return
              setError(err instanceof Error ? err.message : String(err))
              setLoading(false)
            },
          )
        }, DETAILS_DEBOUNCE_MS)
    return () => {
      stale = true
      if (timer !== undefined) clearTimeout(timer)
    }
  }, [host, root, prefix, sha, ready])

  // Which branches have it: a walk over every ref, so only once the
  // selection rests, and again only when the refs moved.
  useEffect(() => {
    setBranches(null)
    if (root === null || sha === null) return
    const key = `${sha}@${signature}`
    const cached = branchCache.current.get(key)
    if (cached) {
      setBranches(cached)
      return
    }
    let stale = false
    const timer = setTimeout(() => {
      readContainingBranches(host, root, sha).then(
        (read) => {
          if (stale) return
          branchCache.current.set(key, read)
          if (branchCache.current.size > DETAILS_CACHE) {
            branchCache.current.delete(branchCache.current.keys().next().value as string)
          }
          setBranches(read)
        },
        () => {},
      )
    }, BRANCHES_DELAY_MS)
    return () => {
      stale = true
      clearTimeout(timer)
    }
  }, [host, root, sha, signature])

  return { details, loading, error, branches }
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

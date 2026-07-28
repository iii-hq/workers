/**
 * The live git-graph — the hero view of the github page. It fetches the working
 * repo's commit DAG with ONE `shell::exec` call (`git log --all …`), lays it out
 * with the pure lane walker (graph-layout.ts), and draws a colored-lane commit
 * graph in an SVG gutter with ref labels, subjects and authors alongside — a
 * Sourcetree/gitk-style graph that refreshes live as the agent works.
 *
 * Live: it binds a tab-scoped subscription to the worker's `github::called`
 * trigger (the same idiom the activity feed uses) and re-fetches the DAG,
 * debounced, whenever a github call lands — plus a manual refresh and a
 * live/paused toggle. Selecting a row opens a detail panel that runs
 * `git show <hash>` and renders it with the shared diff renderer.
 *
 * v1 is local-only (the shell worker's default working dir). `enrichWithGithub`
 * is the typed seam for a v2 github PR/CI overlay — a no-op today.
 */

import {
  Badge,
  Button,
  EmptyState,
  ErrorBoundary,
  type Host,
  Skeleton,
  StatusDot,
  StatusPanel,
} from '@iii-dev/console-ui'
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useGithubCalled } from './events'
import { formatRelative } from './format'
import { type Commit, type GraphEdge, type GraphLayout, layout } from './graph-layout'
import { AlertCircle, FolderGit2, RefreshCw } from './icons'
import { ResultView } from './result-views'

/** Per-tab handler id for the graph's live refresh — distinct from the activity
 *  feed's (`iii::github-ui::called`) so the two subscriptions never collide.
 *  The `iii::` prefix keeps per-event invocations out of the trace feed. */
const GRAPH_EVENTS_FN = 'iii::github-ui::graph-called'
/** Newest-first cap: one shell call, bounded output, bounded layout work. */
const MAX_COMMITS = 300
/** Unit separator between `git log` format fields (matches %x1f). */
const US = '\u001f'
/** Coalesce a burst of github calls into one refetch. */
const REFRESH_DEBOUNCE_MS = 500

/* geometry (px) */
const ROW_H = 26
const COL_W = 16
const NODE_R = 4.5
const PAD_X = 12

/** Wire shape of `shell::exec` (shell/src/functions/types.rs ExecResponse). */
interface ExecResponse {
  exit_code: number | null
  stdout: string
  stderr: string
  stdout_truncated: boolean
}

type Status = 'loading' | 'ready' | 'error'

/**
 * v2 seam — overlay github PR/CI status onto the local commits (map a commit
 * sha to its open PR, checks conclusion, …) so the graph can badge PR/CI state.
 * Typed no-op today; wired into the fetch pipeline so v2 only fills the body.
 */
export function enrichWithGithub(commits: Commit[], _host: Host): Commit[] {
  // TODO(v2): fetch via github::pr::* / github::api and annotate each commit.
  return commits
}

/** Split `git log --format=%H%x1f%P%x1f%D%x1f%an%x1f%ct%x1f%s` output into commits. */
function parseLog(stdout: string): Commit[] {
  const out: Commit[] = []
  for (const line of stdout.split('\n')) {
    if (!line) continue
    const parts = line.split(US)
    if (parts.length < 6) continue
    const [hash, parentField, refField, author, timeField] = parts
    // Subject can (rarely) contain the separator; rejoin the tail.
    const subject = parts.slice(5).join(US)
    out.push({
      hash,
      parents: parentField.trim() ? parentField.trim().split(' ').filter(Boolean) : [],
      refs: parseRefs(refField),
      author,
      time: Number(timeField) || 0,
      subject,
    })
  }
  return out
}

/** `%D` is a comma list: `HEAD -> main, origin/feature, tag: v1.0`. */
function parseRefs(field: string): string[] {
  return field
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
}

interface RefLabel {
  kind: 'head' | 'tag' | 'branch'
  name: string
}

/** Max ref-label chars before ellipsis, so a long branch name can't wrap the
 *  row or crowd out the subject. */
const REF_MAX = 22

function clampRef(name: string): string {
  return name.length > REF_MAX ? `${name.slice(0, REF_MAX - 1)}…` : name
}

/**
 * Classify raw refs into badge-able labels: HEAD pointer split out, the
 * redundant `origin/` prefix dropped, and local/remote duplicates of the same
 * branch collapsed to one badge (so `main` + `origin/main` render once). Keeps
 * the label list short enough that badges stay single-line.
 */
function refLabels(refs: string[]): RefLabel[] {
  const labels: RefLabel[] = []
  const seen = new Set<string>()
  const pushBranch = (branch: string) => {
    const short = branch.replace(/^origin\//, '')
    const key = `b:${short}`
    if (seen.has(key)) return
    seen.add(key)
    labels.push({ kind: 'branch', name: clampRef(short) })
  }
  const pushHead = () => {
    if (seen.has('HEAD')) return
    seen.add('HEAD')
    labels.push({ kind: 'head', name: 'HEAD' })
  }
  for (const raw of refs) {
    if (raw.startsWith('tag: ')) {
      const name = raw.slice(5)
      const key = `t:${name}`
      if (seen.has(key)) continue
      seen.add(key)
      labels.push({ kind: 'tag', name: clampRef(name) })
    } else if (raw.includes(' -> ')) {
      const [head, branch] = raw.split(' -> ')
      if (head === 'HEAD') pushHead()
      pushBranch(branch)
    } else if (raw === 'HEAD') {
      pushHead()
    } else {
      pushBranch(raw)
    }
  }
  return labels
}

/**
 * Fetch + parse the DAG, and keep it live. Binds ONE `github::called`
 * subscription for the component's lifetime; each event schedules a debounced
 * refetch when live. The binding + timer are torn down on unmount.
 */
function useGitGraph(host: Host) {
  const [commits, setCommits] = useState<Commit[]>([])
  const [status, setStatus] = useState<Status>('loading')
  const [error, setError] = useState<string>('')
  const [live, setLive] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const liveRef = useRef(live)
  liveRef.current = live
  const inFlight = useRef(false)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)

  const fetchDag = useCallback(async () => {
    if (inFlight.current) return
    inFlight.current = true
    setRefreshing(true)
    try {
      const res = await host.iii.trigger<ExecResponse>('shell::exec', {
        command: 'git',
        args: ['log', '--all', '--format=%H%x1f%P%x1f%D%x1f%an%x1f%ct%x1f%s', `--max-count=${MAX_COMMITS}`],
      })
      if (res.exit_code !== 0) {
        setStatus('error')
        setError(res.stderr.trim() || `git exited ${res.exit_code}`)
        return
      }
      setCommits(enrichWithGithub(parseLog(res.stdout), host))
      setStatus('ready')
      setError('')
    } catch (e) {
      setStatus('error')
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      inFlight.current = false
      setRefreshing(false)
    }
  }, [host])

  useEffect(() => {
    void fetchDag()
    return () => {
      if (timer.current) clearTimeout(timer.current)
    }
  }, [fetchDag])

  useGithubCalled(host, GRAPH_EVENTS_FN, () => {
    if (!liveRef.current) return
    if (timer.current) clearTimeout(timer.current)
    timer.current = setTimeout(() => void fetchDag(), REFRESH_DEBOUNCE_MS)
  })

  return { commits, status, error, live, setLive, refreshing, refresh: fetchDag }
}

export function GitGraph({ host }: { host: Host }) {
  const { commits, status, error, live, setLive, refreshing, refresh } = useGitGraph(host)
  const [selected, setSelected] = useState<string | null>(null)
  const graph = useMemo(() => layout(commits), [commits])

  return (
    <div className="gh-graph-page">
      <div className="gh-head">
        <div>
          <div className="gh-title">git graph</div>
          <div className="gh-sub">
            {status === 'ready'
              ? `${commits.length} commit${commits.length === 1 ? '' : 's'} across all refs`
              : status === 'error'
                ? 'could not read the repository'
                : 'reading the working repository'}
          </div>
        </div>
        <div className="gh-head-actions">
          <Button variant="ghost" size="sm" aria-pressed={live} onClick={() => setLive((v) => !v)}>
            <StatusDot tone={live ? 'accent' : 'warn'} pulse={live} aria-hidden />
            {live ? 'live' : 'paused'}
          </Button>
          <Button variant="ghost" size="sm" disabled={refreshing} onClick={() => void refresh()}>
            <RefreshCw size={13} aria-hidden />
            refresh
          </Button>
        </div>
      </div>

      {status === 'error' ? (
        <div className="gh-graph-body">
          <StatusPanel
            variant="alert"
            icon={<AlertCircle size={16} aria-hidden />}
            headline="git log failed"
            detail={error || 'the shell worker returned no output'}
          />
        </div>
      ) : status === 'loading' && commits.length === 0 ? (
        <div className="gh-graph-body gh-graph-skeleton">
          {Array.from({ length: 8 }, (_, i) => (
            <Skeleton key={i} style={{ height: `${ROW_H - 8}px`, width: '100%' }} />
          ))}
        </div>
      ) : commits.length === 0 ? (
        <EmptyState
          icon={GitGraphIcon}
          title="no commits yet"
          description="this repository has no history to graph — the graph refreshes automatically as commits land."
        />
      ) : (
        <div className="gh-graph-split">
          <GraphCanvas graph={graph} selected={selected} onSelect={setSelected} />
          {selected ? <CommitDetail host={host} hash={selected} onClose={() => setSelected(null)} /> : null}
        </div>
      )}
    </div>
  )
}

const GitGraphIcon = (p: { className?: string }) => <FolderGit2 size={28} {...p} />

/* ── the SVG gutter + aligned commit rows ────────────────────────────────── */

function GraphCanvas({
  graph,
  selected,
  onSelect,
}: {
  graph: GraphLayout
  selected: string | null
  onSelect: (hash: string) => void
}) {
  const gutter = PAD_X + Math.max(1, graph.columns) * COL_W
  const height = graph.rows.length * ROW_H
  const x = (col: number) => PAD_X + col * COL_W
  const y = (row: number) => row * ROW_H + ROW_H / 2

  // The lane edges depend only on the layout, never on selection — memoize them
  // so selecting a row doesn't rebuild every `<path>`.
  const edges = useMemo(
    () =>
      graph.edges.map((e, i) => (
        <path
          key={i}
          className={`gh-lane-c${e.color}`}
          d={edgePath(e, x, y)}
          fill="none"
          stroke="currentColor"
          strokeWidth={1.5}
        />
      )),
    [graph],
  )

  return (
    <div className="gh-graph-scroll">
      <svg className="gh-graph-svg" width={gutter} height={height} viewBox={`0 0 ${gutter} ${height}`} aria-hidden>
        {edges}
        {graph.rows.map((r) => (
          <g key={r.commit.hash} className={`gh-lane-c${r.color}`}>
            {r.isRefTip ? (
              <circle
                cx={x(r.column)}
                cy={y(r.row)}
                r={NODE_R + 2}
                fill="none"
                stroke="currentColor"
                strokeWidth={1.5}
              />
            ) : null}
            <circle
              cx={x(r.column)}
              cy={y(r.row)}
              r={NODE_R}
              fill="currentColor"
              className={selected === r.commit.hash ? 'gh-node-selected' : undefined}
            />
          </g>
        ))}
      </svg>

      <div className="gh-graph-rows">
        {graph.rows.map((r) => (
          <CommitRow
            key={r.commit.hash}
            commit={r.commit}
            gutter={gutter}
            selected={selected === r.commit.hash}
            onSelect={onSelect}
          />
        ))}
      </div>
    </div>
  )
}

function edgePath(e: GraphEdge, x: (c: number) => number, y: (r: number) => number): string {
  const x0 = x(e.fromColumn)
  const y0 = y(e.fromRow)
  const x1 = x(e.toColumn)
  const y1 = y(e.toRow)
  if (e.fromColumn === e.toColumn) return `M ${x0} ${y0} L ${x1} ${y1}`
  const t = Math.min(ROW_H, y1 - y0)
  if (e.reused) {
    // rejoin: straight down the child's lane, curve into the parent at the bottom
    const yb = y1 - t
    return `M ${x0} ${y0} L ${x0} ${yb} C ${x0} ${yb + t / 2} ${x1} ${y1 - t / 2} ${x1} ${y1}`
  }
  // fan-out: curve into the new lane at the top, then straight down to the parent
  const yb = y0 + t
  return `M ${x0} ${y0} C ${x0} ${y0 + t / 2} ${x1} ${yb - t / 2} ${x1} ${yb} L ${x1} ${y1}`
}

const CommitRow = memo(function CommitRow({
  commit,
  gutter,
  selected,
  onSelect,
}: {
  commit: Commit
  gutter: number
  selected: boolean
  onSelect: (hash: string) => void
}) {
  const labels = refLabels(commit.refs)
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onSelect(commit.hash)
    }
  }
  return (
    <div
      className={`gh-graph-row${selected ? ' selected' : ''}`}
      style={{ height: `${ROW_H}px`, paddingLeft: `${gutter}px` }}
      role="button"
      tabIndex={0}
      aria-pressed={selected}
      onClick={() => onSelect(commit.hash)}
      onKeyDown={onKeyDown}
    >
      {labels.length > 0 ? (
        <span className="gh-graph-refs">
          {labels.map((l, i) => (
            <Badge key={i} variant={l.kind === 'head' ? 'accent' : l.kind === 'tag' ? 'warn' : 'default'}>
              {l.name}
            </Badge>
          ))}
        </span>
      ) : null}
      <span className="gh-graph-subject">{commit.subject || '(no message)'}</span>
      <span className="gh-graph-meta">
        <span className="gh-graph-hash">{commit.hash.slice(0, 7)}</span>
        <span className="gh-graph-author">{commit.author}</span>
        <span className="gh-graph-time">{commit.time ? formatRelative(Date.now() / 1000 - commit.time) : ''}</span>
      </span>
    </div>
  )
})

/* ── commit detail (git show → shared diff renderer) ─────────────────────── */

function CommitDetail({ host, hash, onClose }: { host: Host; hash: string; onClose: () => void }) {
  const [text, setText] = useState<string>('')
  const [truncated, setTruncated] = useState(false)
  const [status, setStatus] = useState<Status>('loading')
  const [error, setError] = useState('')

  useEffect(() => {
    let cancelled = false
    setStatus('loading')
    host.iii
      .trigger<ExecResponse>('shell::exec', {
        command: 'git',
        args: ['show', '--stat', '--patch', '--no-color', hash],
      })
      .then((res) => {
        if (cancelled) return
        if (res.exit_code !== 0) {
          setStatus('error')
          setError(res.stderr.trim() || `git exited ${res.exit_code}`)
          return
        }
        setText(res.stdout)
        setTruncated(res.stdout_truncated)
        setStatus('ready')
      })
      .catch((e) => {
        if (cancelled) return
        setStatus('error')
        setError(e instanceof Error ? e.message : String(e))
      })
    return () => {
      cancelled = true
    }
  }, [host, hash])

  return (
    <div className="gh-graph-detail">
      <div className="gh-graph-detail-head">
        <span className="gh-graph-hash">{hash.slice(0, 12)}</span>
        <Button variant="ghost" size="sm" onClick={onClose} aria-label="close commit detail">
          close
        </Button>
      </div>
      <div className="gh-graph-detail-body">
        {status === 'loading' ? (
          <Skeleton style={{ height: '120px', width: '100%' }} />
        ) : status === 'error' ? (
          <div className="gh-rv-empty gh-rv-errtext">{error || 'could not load this commit'}</div>
        ) : (
          <ErrorBoundary fallback={() => <div className="gh-rv-empty">could not render this commit</div>}>
            <ResultView kind="diff" preview={{ diff: text, truncated }} ok />
          </ErrorBoundary>
        )}
      </div>
    </div>
  )
}

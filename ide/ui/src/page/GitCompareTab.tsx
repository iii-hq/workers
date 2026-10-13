/* "Compare with" as an editor tab: two logs, one over the
   other, of the commits each branch has that the other lacks, each with
   its own filters (text or hash, user, date, paths) and the picked
   commit's files and message beside it. */

import type { Host } from '@iii-dev/console-ui'
import { copyText, errorMessage } from '@iii-dev/console-ui/format'
import { TriangleAlert } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { glyphColor, glyphOf } from './CommitGraph'
import { type FilesView, GitCommitDetails } from './GitCommitDetails'
import { GitCommitList } from './GitCommitList'
import { gitCommitPatch } from './git-actions'
import {
  type CommitDetails,
  type CommitFile,
  type LogFilter,
  type LogRef,
  labelsBySha,
  shortRef,
} from './git-log-window'
import { useCommitDetails, useGitLog } from './use-git-log'
import { useWorktreeEpoch, useWorktreeOps, type WorktreeOps, type WorktreesPage } from './use-worktree-ops'
import { worktreeAt } from './worktrees'

const NO_BRANCHES: ReadonlyArray<{ id: string; name: string }> = []
const NO_SEEDS: readonly string[] = []
const NO_RINGS: ReadonlyMap<string, 'here' | 'worktree'> = new Map()
const FILES_VIEW: FilesView = { height: null, grouped: true, info: true, preview: false }
const ignore = () => undefined

export interface GitCompareTabProps {
  host: Host
  root: string
  page: WorktreesPage
  /** Full ref names: the branch compared, and the one it is compared with. */
  refName: string
  against: string
  onOpenCommitFile(file: CommitFile, details: CommitDetails, pin?: boolean): void
  onOpenCompareFile(file: CommitFile, ref: string, from?: string): void
  onOpenWorkingFile(rel: string): void
  onOpenRevision(file: CommitFile, sha: string): void
}

export function GitCompareTab(props: GitCompareTabProps) {
  const { host, root, page, refName, against } = props
  const ops = useWorktreeOps(host, root, page, true, 'view')
  return (
    <div className="shui-git-window shui-compare-tab">
      {ops.note !== null ? <p className="shui-git-note-line">{ops.note}</p> : null}
      <ComparePane {...props} ops={ops} tip={refName} notIn={against} />
      <ComparePane {...props} ops={ops} tip={against} notIn={refName} />
    </div>
  )
}

function ComparePane({
  host,
  root,
  ops,
  tip,
  notIn,
  onOpenCommitFile,
  onOpenCompareFile,
  onOpenWorkingFile,
  onOpenRevision,
}: GitCompareTabProps & { ops: WorktreeOps; tip: string; notIn: string }) {
  const epoch = useWorktreeEpoch()
  const [filter, setFilter] = useState<LogFilter>({})
  // The comparison stays under whatever the filters say.
  const merged = useMemo(() => ({ ...filter, notIn }), [filter, notIn])
  const log = useGitLog(host, root, epoch, true, merged, tip)
  const snapshot = log.snapshot
  const [selected, setSelected] = useState<string | null>(null)
  const details = useCommitDetails(host, root, snapshot, selected)
  const [view, setView] = useState(FILES_VIEW)
  const [hint, setHint] = useState<string | null>(null)
  const target = ops.list?.defaultBranch ?? null
  const remotes = useMemo(
    () => new Set((snapshot?.refs ?? []).filter((ref) => ref.kind === 'remote').map((ref) => ref.name.split('/')[0])),
    [snapshot],
  )
  const labels = useMemo(() => (snapshot === null ? new Map<string, LogRef[]>() : labelsBySha(snapshot)), [snapshot])
  const glyph = useCallback((name: string | null) => glyphOf(name, target, remotes), [target, remotes])
  const here = ops.list === null ? null : worktreeAt(ops.list.worktrees, root)
  const onView = useCallback((patch: Partial<FilesView>) => setView((current) => ({ ...current, ...patch })), [])
  const copyPatch = useCallback(
    (sha: string, paths: string[]) => {
      setHint(null)
      gitCommitPatch(host, root, sha, paths)
        .then(copyText)
        .then(
          (copied) => {
            if (!copied) setHint('copy as patch failed: the clipboard refused it')
          },
          (err: unknown) => setHint(`copy as patch failed: ${errorMessage(err)}`),
        )
    },
    [host, root],
  )
  // A file's history, within the comparison.
  const showHistory = useCallback((paths: string[], sha: string) => {
    setFilter((previous) => ({ ...previous, paths }))
    setSelected(sha)
  }, [])

  return (
    <section className="shui-compare-pane" aria-label={`Commits in ${shortRef(tip)} not in ${shortRef(notIn)}`}>
      <div className="shui-compare-log">
        <p className="shui-compare-banner">
          <TriangleAlert aria-hidden />
          <span>
            Commits that exist in <b>{shortRef(tip)}</b> but don't exist in <b>{shortRef(notIn)}</b>
          </span>
        </p>
        {hint !== null ? <p className="shui-git-note-line warn">{hint}</p> : null}
        <GitCommitList
          log={log}
          filter={filter}
          onFilter={setFilter}
          branchLabel={null}
          branches={NO_BRANCHES}
          onBranch={ignore}
          branchFilter={false}
          seeds={NO_SEEDS}
          headColor={glyphColor(glyph(tip.replace(/^refs\/heads\//, '')))}
          prefix={snapshot?.prefix ?? ''}
          labels={labels}
          remotes={remotes}
          glyph={glyph}
          rings={NO_RINGS}
          selected={selected}
          onSelect={setSelected}
          onAct={ignore}
          onMenu={ignore}
        />
      </div>
      <div className="shui-compare-details">
        <GitCommitDetails
          host={host}
          root={root}
          state={details}
          selected={log.commits.length === 0 && !log.loading ? null : selected}
          shallow={snapshot?.shallow ?? false}
          onOpenFile={onOpenCommitFile}
          prefix={snapshot?.prefix ?? ''}
          top={here?.path ?? null}
          onSelectCommit={setSelected}
          view={view}
          onView={onView}
          busy={ops.busy}
          onCompare={onOpenCompareFile}
          onEditSource={onOpenWorkingFile}
          onCommitFiles={ops.commitFiles}
          onCopyPatch={copyPatch}
          onHistory={showHistory}
          onOpenRevision={onOpenRevision}
        />
      </div>
    </section>
  )
}

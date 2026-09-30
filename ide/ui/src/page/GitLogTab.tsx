/* The Git window's Log tab, laid out like WebStorm's log. It has three
   panes side by side: the branches tree, the commit graph and the selected
   commit, with the branch actions on a rail at the left edge.

   Picking a branch in the tree limits the log to it and selects its tip,
   so the commit pane fills at once. Enter or a double click on a branch
   opens it worktrunk's way: the IDE and the chat move into its worktree,
   and one is made when it has none.

   A branch's menu has WebStorm's verbs, worktrunk's way (nothing is ever
   checked out in place): a new branch or worktree from it, its diff with
   the working tree (in the right pane), Update from its upstream, Push,
   its tracked branch's own verbs, Rename.

   A narrow pane shows one pane at a time and drills in: Branches, then a
   branch's Commits, then one Commit, with Back in a header above and the
   branch actions in a bar along the bottom. */

import type { Host } from '@iii-dev/console-ui'
import { ConfirmDialog, EmptyState } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { usePaneState, useSplitDrag } from '@iii-dev/console-ui/hooks'
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  ChevronLeft,
  ChevronsDownUp,
  Cloud,
  CloudDownload,
  Copy,
  FileDiff,
  FolderInput,
  FolderPlus,
  GitBranchPlus,
  GitMerge,
  PenLine,
  Trash2,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ActionRail, type GitAction, menuItems } from './ActionRail'
import { glyphColor, glyphOf } from './CommitGraph'
import { useContextMenu } from './ContextMenu'
import { GitBranchTree } from './GitBranchTree'
import { type FilesView, GitCommitDetails } from './GitCommitDetails'
import { GitCommitList } from './GitCommitList'
import { GitCompareFiles } from './GitCompareFiles'
import { git, gitCommitPatch } from './git-actions'
import {
  type CommitDetails,
  type CommitFile,
  type LogCommit,
  type LogFilter,
  type LogRef,
  labelsBySha,
  type RefTreeNode,
  refsTree,
} from './git-log-window'
import { useCommitDetails, useGitLog, useWorkingDiff } from './use-git-log'
import { useWorktreeEpoch, type WorktreeOps } from './use-worktree-ops'
import { BranchNameForm, type MergeDraft, MergeForm, NewWorktreeForm } from './WorktreeForms'
import { type PushTarget, pushTarget, type Worktree, worktreeAt } from './worktrees'

interface Layout {
  tree: number
  details: number
  /** How the commit's files show. */
  files: FilesView
  expanded: string[]
}
const LAYOUT: Layout = {
  tree: 240,
  details: 320,
  files: { height: null, grouped: true, info: true },
  expanded: ['local'],
}

/** The narrow pane's drill-in steps. */
type Stage = 'branches' | 'commits' | 'commit'

function normalize(raw: unknown): Layout {
  const value = (raw ?? {}) as Partial<Layout>
  const width = (n: unknown, fallback: number) =>
    typeof n === 'number' && Number.isFinite(n) ? Math.min(900, Math.max(160, Math.round(n))) : fallback
  return {
    tree: width(value.tree, LAYOUT.tree),
    details: width(value.details, LAYOUT.details),
    files: filesView(value.files),
    expanded: Array.isArray(value.expanded)
      ? value.expanded.filter((id): id is string => typeof id === 'string')
      : LAYOUT.expanded,
  }
}

function filesView(raw: unknown): FilesView {
  const value = (raw ?? {}) as Partial<FilesView>
  return {
    height:
      typeof value.height === 'number' && Number.isFinite(value.height) ? Math.max(48, Math.round(value.height)) : null,
    grouped: value.grouped !== false,
    info: value.info !== false,
  }
}

function flatten(nodes: readonly RefTreeNode[], into = new Map<string, RefTreeNode>()): Map<string, RefTreeNode> {
  for (const node of nodes) {
    into.set(node.id, node)
    if (node.kind === 'section' || node.kind === 'folder') flatten(node.children, into)
  }
  return into
}

/** A drag handle between two panes; `side` says which pane it sizes. */
function Sash({
  label,
  width,
  side,
  onWidth,
}: {
  label: string
  width: number
  side: 'left' | 'right'
  onWidth(next: number): void
}) {
  const drag = useSplitDrag<number>({
    horizontal: true,
    begin: () => width,
    move: (origin, delta) => onWidth(origin + (side === 'left' ? delta : -delta)),
    step: (direction) => onWidth(width + (side === 'left' ? direction : -direction) * 16),
  })
  return (
    // biome-ignore lint/a11y/useSemanticElements: an interactive range separator, not a thematic break
    <div
      role="separator"
      tabIndex={0}
      className="shui-git-sash"
      aria-label={label}
      aria-orientation="vertical"
      aria-valuemin={160}
      aria-valuemax={900}
      aria-valuenow={width}
      {...drag}
    />
  )
}

export function GitLogTab({
  host,
  root,
  ops,
  active,
  paneKey,
  focusBranch,
  narrow = false,
  onOpenCommitFile,
  onOpenCompareFile,
  onOpenWorkingFile,
}: {
  host: Host
  root: string
  ops: WorktreeOps
  /** The tab shows: reads run only then. */
  active: boolean
  paneKey: string
  /** "Show in Log" from the Worktrees tab: the branch to select. */
  focusBranch: { name: string; seq: number } | null
  /** One pane at a time, drilled into. */
  narrow?: boolean
  onOpenCommitFile(file: CommitFile, details: CommitDetails, pin?: boolean): void
  /** Opens a file beside its working copy, as it is at `ref`. */
  onOpenCompareFile(file: CommitFile, ref: string, from?: string): void
  onOpenWorkingFile(rel: string): void
}) {
  const epoch = useWorktreeEpoch()
  const [filter, setFilter] = useState<LogFilter>({})
  const [treeSel, setTreeSel] = useState<string | null>(null)
  const [commitSel, setCommitSel] = useState<string | null>(null)
  const [stored, setStored] = usePaneState<Layout>(`iii::shell-ui::git::${paneKey}`, LAYOUT)
  // Normalized once per stored value: a fresh `expanded` array each render
  // would rebuild the tree's open set, and every row with it.
  const layout = useMemo(() => normalize(stored), [stored])
  const expanded = useMemo(() => new Set(layout.expanded), [layout.expanded])
  const [creating, setCreating] = useState<{ from: string; label: string } | null>(null)
  const [merging, setMerging] = useState<{ branch: string; wt: Worktree | null; draft: MergeDraft } | null>(null)
  // A new branch's name, or a branch's new one.
  const [naming, setNaming] = useState<{ kind: 'branch' | 'rename'; from: string; label: string } | null>(null)
  const [pushing, setPushing] = useState<{ branch: string; target: PushTarget } | null>(null)
  // "Show diff with working tree": the right pane lists what differs.
  const [comparing, setComparing] = useState<{ ref: string; label: string } | null>(null)
  // A look-up an action needed first failed (where a push would go).
  const [hint, setHint] = useState<string | null>(null)
  const menu = useContextMenu()
  const [stage, setStage] = useState<Stage>('commits')
  // The keyboard follows a drill into its stage, unless focus is elsewhere.
  const paneRef = useRef<HTMLDivElement>(null)
  const lastStage = useRef(stage)
  useEffect(() => {
    if (lastStage.current === stage) return
    lastStage.current = stage
    const pane = paneRef.current
    const focused = document.activeElement
    if (pane === null || (focused !== null && focused !== document.body && !pane.contains(focused))) return
    requestAnimationFrame(() => pane.querySelector<HTMLElement>('[data-git-focus]')?.focus())
  }, [stage])

  const target = ops.list?.defaultBranch ?? null
  // A ref's node id is `ref:<full name>`: the log follows it (or HEAD); a
  // group leaves it on every branch.
  const tipRef = treeSel?.startsWith('ref:') ? treeSel.slice(4) : treeSel === 'head' ? 'HEAD' : null
  const log = useGitLog(host, root, epoch, active, filter, tipRef)
  // A branch picked in the tree or the Branch menu ends a "History up to here".
  const selectNode = useCallback((id: string | null) => {
    setFilter((prev) => (prev.upTo === undefined ? prev : { ...prev, upTo: undefined }))
    setTreeSel(id)
  }, [])
  const details = useCommitDetails(host, root, log.snapshot, commitSel)
  const working = useWorkingDiff(host, root, log.snapshot?.prefix ?? null, comparing?.ref ?? null, epoch)
  const snapshot = log.snapshot

  const nodes = useMemo(() => (snapshot === null ? [] : refsTree(snapshot, target)), [snapshot, target])
  const nodeById = useMemo(() => flatten(nodes), [nodes])
  const parentOf = useMemo(() => {
    const out = new Map<string, string>()
    const walk = (list: readonly RefTreeNode[], parent: string | null) => {
      for (const node of list) {
        if (parent !== null) out.set(node.id, parent)
        if (node.kind === 'section' || node.kind === 'folder') walk(node.children, node.id)
      }
    }
    walk(nodes, null)
    return out
  }, [nodes])
  const selectedNode = treeSel === null ? null : (nodeById.get(treeSel) ?? null)
  const branchChoices = useMemo(() => {
    const out: Array<{ id: string; name: string }> = []
    for (const node of nodeById.values()) {
      if (node.kind === 'head') out.unshift({ id: node.id, name: 'HEAD' })
      else if (node.kind === 'ref' && node.ref.kind === 'local') out.push({ id: node.id, name: node.ref.name })
    }
    return out
  }, [nodeById])
  const branchLabel = filter.upTo
    ? `up to ${filter.upTo.slice(0, 7)}`
    : selectedNode?.kind === 'ref'
      ? selectedNode.ref.name
      : selectedNode?.kind === 'head'
        ? 'HEAD'
        : null
  const remotes = useMemo(
    () => new Set((snapshot?.refs ?? []).filter((ref) => ref.kind === 'remote').map((ref) => ref.name.split('/')[0])),
    [snapshot],
  )
  const labels = useMemo(() => (snapshot === null ? new Map<string, LogRef[]>() : labelsBySha(snapshot)), [snapshot])
  const here = ops.list === null ? null : worktreeAt(ops.list.worktrees, root)
  const rings = useMemo(() => {
    const out = new Map<string, 'here' | 'worktree'>()
    for (const wt of ops.list?.worktrees ?? []) {
      if (wt.head) out.set(wt.head, wt.path === here?.path ? 'here' : 'worktree')
    }
    return out
  }, [ops.list, here?.path])

  const setExpanded = (id: string, open: boolean) =>
    setStored((prev) => {
      const current = normalize(prev)
      const next = new Set(current.expanded)
      if (open) next.add(id)
      else next.delete(id)
      return { ...current, expanded: [...next] }
    })

  // The selected commit. Picking a branch selects its tip at once, from the
  // refs: its page may not be read yet. A new list keeps the selection while
  // it is listed, else takes the IDE's HEAD, else the newest; a list that
  // only grew keeps even an unlisted one (a parent picked in the details).
  const head = snapshot?.head ?? null
  const tipSha =
    tipRef === null
      ? null
      : tipRef === 'HEAD'
        ? head
        : (snapshot?.refs.find((ref) => ref.fullName === tipRef)?.sha ?? null)
  const lastTip = useRef<string | null>(null)
  useEffect(() => {
    if (lastTip.current === tipRef) return
    lastTip.current = tipRef
    if (tipSha !== null) setCommitSel(tipSha)
  }, [tipRef, tipSha])
  const lastList = useRef<readonly LogCommit[]>([])
  useEffect(() => {
    const before = lastList.current
    const list = log.commits
    lastList.current = list
    if (list.length === 0) return
    const grew =
      before.length > 0 &&
      list.length > before.length &&
      list[0].sha === before[0].sha &&
      list[before.length - 1].sha === before[before.length - 1].sha
    if (grew) return
    setCommitSel((current) =>
      current !== null && list.some((commit) => commit.sha === current)
        ? current
        : head !== null && list.some((commit) => commit.sha === head)
          ? head
          : list[0].sha,
    )
  }, [log.commits, head])

  // A branch that went away (deleted, or not in this repository) leaves the
  // log on every branch; another repository starts from every branch too.
  useEffect(() => {
    if (snapshot !== null) setTreeSel((current) => (current !== null && !nodeById.has(current) ? null : current))
  }, [snapshot, nodeById])
  const repo = ops.list?.worktrees[0]?.path ?? null
  const lastRepo = useRef(repo)
  useEffect(() => {
    if (repo === null || lastRepo.current === repo) return
    const moved = lastRepo.current !== null
    lastRepo.current = repo
    if (moved) setTreeSel(null)
  }, [repo])

  // Where the IDE's HEAD meets the branch shown: the log fades the commits
  // HEAD lacks, reaching down from there (from HEAD when the log has it).
  const basesFor = tipSha !== null && head !== null && tipRef !== 'HEAD' ? `${tipSha} ${head}` : null
  const [bases, setBases] = useState<{ key: string; shas: string[] } | null>(null)
  useEffect(() => {
    if (basesFor === null) return
    let live = true
    const [tip, at] = basesFor.split(' ')
    git(host, root, ['merge-base', '--all', tip, at]).then(
      (out) => {
        if (live) setBases({ key: basesFor, shas: out.stdout.split('\n').filter((line) => line !== '') })
      },
      () => {
        if (live) setBases({ key: basesFor, shas: [] })
      },
    )
    return () => {
      live = false
    }
  }, [host, root, basesFor])
  const seeds = useMemo(
    () => (basesFor === null ? (head === null ? [] : [head]) : bases?.key === basesFor ? bases.shas : []),
    [basesFor, head, bases],
  )

  // A local branch selected, with its groups open: "Show in Log", or one
  // just made or renamed (picked before its refs are read).
  const revealBranch = (name: string) => {
    const slash = name.indexOf('/')
    setExpanded('local', true)
    if (slash > 0) setExpanded(`local/${name.slice(0, slash)}`, true)
    setTreeSel(`ref:refs/heads/${name}`)
  }
  useEffect(() => {
    if (focusBranch !== null) revealBranch(focusBranch.name)
  }, [focusBranch?.seq])

  const worktreeOf = (ref: LogRef): Worktree | null =>
    ref.worktree === undefined ? null : (ops.list?.worktrees.find((wt) => wt.path === ref.worktree) ?? null)
  const localNameOf = (ref: LogRef): string =>
    ref.kind === 'remote' ? ref.name.slice(ref.name.indexOf('/') + 1) : ref.name

  /** Open a branch worktrunk's way: its worktree, made when it has none. */
  const openRef = (ref: LogRef) => {
    const local = snapshot?.refs.find((candidate) => candidate.kind === 'local' && candidate.name === localNameOf(ref))
    if (local !== undefined) {
      const wt = worktreeOf(local)
      if (wt !== null) ops.switchTo(wt)
      else ops.create(local.name)
    } else if (ref.kind === 'remote') {
      ops.create(localNameOf(ref), undefined, ref.fullName)
    }
  }
  const openMerge = (ref: LogRef) => {
    const wt = worktreeOf(ref)
    setCreating(null)
    setNaming(null)
    setMerging({ branch: ref.name, wt, draft: { path: `branch:${ref.name}`, squash: true, message: '' } })
    const message = wt !== null ? ops.mergeMessage(wt) : ops.branchMergeMessage(ref.name)
    void message.then((text) =>
      setMerging((current) =>
        current?.branch === ref.name && current.draft.message === ''
          ? { ...current, draft: { ...current.draft, message: text } }
          : current,
      ),
    )
  }
  const startNew = (from: string, label: string) => {
    setMerging(null)
    setNaming(null)
    setCreating({ from, label })
  }
  const startNaming = (kind: 'branch' | 'rename', from: string, label: string) => {
    setMerging(null)
    setCreating(null)
    setNaming({ kind, from, label })
  }
  const startPush = (branch: string) => {
    const list = ops.list
    if (list === null) return
    setHint(null)
    pushTarget(host, list, branch).then(
      (target) => setPushing({ branch, target }),
      (err: unknown) => setHint(`push failed: ${errorMessage(err)}`),
    )
  }
  const compareWith = (ref: string) => {
    setComparing({ ref, label: ref })
    if (narrow) setStage('commit')
  }

  const busyWhy = ops.busy ? 'another worktree operation is running' : null
  const refActions = (node: RefTreeNode | null): GitAction[] => {
    const ref = node?.kind === 'ref' ? node.ref : node?.kind === 'head' ? node.ref : null
    const wt = ref === null ? null : worktreeOf(ref)
    const isLocal = ref?.kind === 'local'
    const inIde = wt !== null && wt.path === here?.path
    // A commit to start from: HEAD by its commit, since `HEAD` would
    // resolve in the repository's main worktree.
    const pointed = node?.kind === 'ref' || node?.kind === 'head'
    const name = node?.kind === 'head' ? 'HEAD' : (ref?.name ?? '')
    const start = node?.kind === 'head' ? head : (ref?.fullName ?? null)
    const noStart = pointed && start === null ? 'HEAD has no commit yet' : null
    const upstream =
      isLocal && ref.upstream !== undefined && !ref.gone
        ? (snapshot?.refs.find((candidate) => candidate.kind === 'remote' && candidate.name === ref.upstream) ?? null)
        : null
    const upNode = upstream === null ? null : (nodeById.get(`ref:${upstream.fullName}`) ?? null)
    return [
      {
        id: 'open',
        group: 'open',
        label: ref?.kind === 'remote' ? `Open as ${localNameOf(ref)}` : 'Open',
        short: 'Open',
        icon: <FolderInput aria-hidden />,
        shortcut: 'Enter',
        primary: true,
        applies: pointed && ref?.kind !== 'tag',
        blocked:
          ref === null || ref.kind === 'tag'
            ? 'select a branch'
            : inIde || node?.kind === 'head'
              ? 'the IDE is on it'
              : busyWhy,
        run: () => {
          if (ref !== null) openRef(ref)
        },
      },
      {
        id: 'new-branch',
        group: 'open',
        label: `New branch from '${name}'…`,
        short: 'Branch',
        icon: <GitBranchPlus aria-hidden />,
        applies: pointed,
        blocked: !pointed ? 'select a branch, a tag or HEAD' : (noStart ?? busyWhy),
        run: () => {
          if (start !== null) startNaming('branch', start, name)
        },
      },
      {
        id: 'new',
        group: 'open',
        label: `New worktree from '${name}'…`,
        short: 'Worktree',
        icon: <FolderPlus aria-hidden />,
        primary: true,
        applies: pointed,
        blocked: !pointed ? 'select a branch, a tag or HEAD' : (noStart ?? busyWhy),
        run: () => {
          if (start !== null) startNew(start, name)
        },
      },
      {
        id: 'diff',
        group: 'compare',
        label: 'Show diff with working tree',
        short: 'Diff',
        icon: <FileDiff aria-hidden />,
        applies: pointed,
        blocked: !pointed ? 'select a branch, a tag or HEAD' : noStart,
        run: () => compareWith(name),
      },
      {
        id: 'update',
        group: 'sync',
        label: 'Update',
        short: 'Update',
        icon: <ArrowDownToLine aria-hidden />,
        primary: true,
        applies: isLocal,
        blocked: !isLocal
          ? 'select a local branch'
          : ref.upstream === undefined
            ? 'it tracks no remote branch'
            : ref.gone
              ? `${ref.upstream} is gone`
              : busyWhy,
        run: () => {
          if (ref !== null) ops.updateBranch(ref.name)
        },
      },
      {
        id: 'push',
        group: 'sync',
        label: 'Push…',
        short: 'Push',
        icon: <ArrowUpFromLine aria-hidden />,
        primary: true,
        applies: isLocal,
        blocked: !isLocal ? 'select a local branch' : remotes.size === 0 ? 'no remote to push to' : busyWhy,
        run: () => {
          if (ref !== null) startPush(ref.name)
        },
      },
      {
        id: 'fetch',
        group: 'sync',
        label: 'Fetch all remotes',
        short: 'Fetch',
        icon: <CloudDownload aria-hidden />,
        in: 'rail',
        blocked: remotes.size === 0 ? 'no remote to fetch' : busyWhy,
        run: ops.fetch,
      },
      {
        id: 'tracked',
        group: 'sync',
        label: `Tracked branch '${upstream?.name ?? ''}'`,
        icon: <Cloud aria-hidden />,
        in: 'menu',
        applies: upNode !== null,
        // The tracked branch's own verbs, as WebStorm lists them.
        items:
          upNode === null
            ? []
            : refActions(upNode).filter((action) => ['open', 'new-branch', 'new', 'diff', 'copy'].includes(action.id)),
        run: () => {},
      },
      {
        id: 'merge',
        group: 'merge',
        label: `Merge into ${target ?? 'the default branch'}`,
        short: 'Merge',
        icon: <GitMerge aria-hidden />,
        applies: isLocal,
        blocked: !isLocal ? 'select a local branch' : ref?.name === target ? `it is ${target}` : busyWhy,
        run: () => {
          if (ref !== null) openMerge(ref)
        },
      },
      {
        id: 'rename',
        group: 'edit',
        label: 'Rename…',
        icon: <PenLine aria-hidden />,
        shortcut: 'F2',
        in: 'menu',
        applies: isLocal,
        blocked: !isLocal
          ? 'select a local branch'
          : ref?.name === target
            ? 'the default branch keeps its name'
            : busyWhy,
        run: () => {
          if (ref !== null) startNaming('rename', ref.name, ref.name)
        },
      },
      {
        id: 'copy',
        group: 'edit',
        label: 'Copy name',
        icon: <Copy aria-hidden />,
        in: 'menu',
        applies: pointed,
        blocked: ref === null ? 'select a branch or a tag' : null,
        run: () => void navigator.clipboard?.writeText(name),
      },
      {
        id: 'delete',
        group: 'delete',
        label: wt !== null ? 'Remove its worktree' : 'Delete',
        short: wt !== null ? 'Remove' : 'Delete',
        icon: <Trash2 aria-hidden />,
        shortcut: 'Delete',
        danger: true,
        applies: isLocal,
        blocked: !isLocal
          ? 'select a local branch'
          : ref?.name === target
            ? 'the default branch stays'
            : wt?.main
              ? 'the main worktree stays'
              : inIde || ref?.current
                ? 'the IDE is on it'
                : busyWhy,
        run: () => {
          if (ref === null) return
          if (wt !== null) ops.askRemove(wt)
          else ops.askDeleteBranch(ref.name)
        },
      },
      {
        id: 'collapse',
        label: 'Collapse all',
        short: 'Collapse',
        icon: <ChevronsDownUp aria-hidden />,
        in: 'rail',
        end: true,
        run: () => setStored((prev) => ({ ...normalize(prev), expanded: [] })),
      },
    ]
  }
  const run = (actions: readonly GitAction[], id: string) => {
    const action = actions.find((candidate) => candidate.id === id)
    if (action && (action.blocked ?? null) === null) action.run()
  }

  const commitActions = (commit: LogCommit): GitAction[] => {
    const refs = (labels.get(commit.sha) ?? []).filter((ref) => ref.kind !== 'tag').slice(0, 3)
    return [
      {
        id: 'copy-hash',
        group: 'copy',
        label: 'Copy hash',
        short: 'Copy',
        icon: <Copy aria-hidden />,
        run: () => void navigator.clipboard?.writeText(commit.sha),
      },
      {
        id: 'new-branch-here',
        group: 'new',
        label: `New branch from ${commit.sha.slice(0, 7)}…`,
        short: 'Branch',
        icon: <GitBranchPlus aria-hidden />,
        blocked: busyWhy,
        run: () => startNaming('branch', commit.sha, commit.sha.slice(0, 7)),
      },
      {
        id: 'new-here',
        group: 'new',
        label: `New worktree from ${commit.sha.slice(0, 7)}…`,
        short: 'Worktree',
        icon: <FolderPlus aria-hidden />,
        blocked: busyWhy,
        run: () => startNew(commit.sha, commit.sha.slice(0, 7)),
      },
      {
        id: 'diff-here',
        group: 'compare',
        label: 'Show diff with working tree',
        short: 'Diff',
        icon: <FileDiff aria-hidden />,
        run: () => compareWith(commit.sha.slice(0, 12)),
      },
      ...refs.map(
        (ref): GitAction => ({
          id: `open:${ref.fullName}`,
          group: 'open',
          label: `Open ${ref.name}`,
          short: 'Open',
          icon: <FolderInput aria-hidden />,
          blocked: busyWhy,
          run: () => openRef(ref),
        }),
      ),
    ]
  }

  const openCommit = (commit: LogCommit) => {
    const current = details.details
    if (current !== null && current.sha === commit.sha) {
      const first = current.files[0]
      if (first !== undefined) onOpenCommitFile(first, current)
    } else {
      setCommitSel(commit.sha)
    }
  }

  const repoGlyph = (name: string | null) => glyphOf(name, target, remotes)

  if (log.notRepo) {
    return (
      <div className="shui-git-pane" data-pane="log">
        <EmptyState title="No repository" description="This folder is not inside a Git repository." />
      </div>
    )
  }
  if (snapshot !== null && snapshot.head === null && log.commits.length === 0 && !log.loading) {
    return (
      <div className="shui-git-pane" data-pane="log">
        <EmptyState title="No commits yet" description="The log fills in with the first commit." />
      </div>
    )
  }

  // With nothing picked in the tree, the rail acts on HEAD's branch (as
  // WebStorm's toolbar does), so it opens live rather than greyed out.
  const railNode = selectedNode ?? nodeById.get('head') ?? null
  // The narrow bar acts on what the stage shows: the commit on its own
  // stage (one Open, for its first branch), else the branch picked.
  const selectedCommit = commitSel === null ? null : (log.commits.find((commit) => commit.sha === commitSel) ?? null)
  const onCommit = narrow && stage === 'commit' && selectedCommit !== null
  const barActions = (): GitAction[] => {
    if (selectedCommit !== null && onCommit) {
      const actions = commitActions(selectedCommit)
      const open = actions.find((action) => action.id.startsWith('open:'))
      return actions.filter((action) => !action.id.startsWith('open:') || action === open)
    }
    return refActions(railNode).filter((action) => !narrow || stage === 'branches' || action.id !== 'collapse')
  }
  const rail = (
    <ActionRail
      label={onCommit ? 'Commit actions' : 'Branch actions'}
      actions={barActions()}
      bar={narrow}
      onMore={(anchor, rest) => menu.open(anchor, menuItems(rest, true))}
    />
  )
  const shows = (pane: Stage) => !narrow || stage === pane
  const pickBranch = (id: string | null) => {
    selectNode(id)
    for (let up = id === null ? undefined : parentOf.get(id); up !== undefined; up = parentOf.get(up)) {
      setExpanded(up, true)
    }
  }
  const drill = narrow ? (
    <div className="shui-git-drill">
      {stage !== 'branches' ? (
        <button
          type="button"
          className="shui-git-back"
          data-git-focus={stage === 'commit' ? '' : undefined}
          onClick={() => setStage(stage === 'commit' ? 'commits' : 'branches')}
        >
          <ChevronLeft aria-hidden />
          {stage === 'commit' ? 'Commits' : 'Branches'}
        </button>
      ) : null}
      <h3 className="shui-git-drill-title">
        {stage === 'branches'
          ? 'Branches'
          : stage === 'commits'
            ? (branchLabel ?? 'All branches')
            : comparing !== null
              ? `${comparing.label} ↔ working tree`
              : `Commit ${commitSel?.slice(0, 7) ?? ''}`}
      </h3>
      {stage === 'branches' ? (
        <button
          type="button"
          className="shui-git-back"
          data-end=""
          onClick={() => {
            selectNode(null)
            setStage('commits')
          }}
        >
          All commits
        </button>
      ) : null}
    </div>
  ) : null

  return (
    <div ref={paneRef} className="shui-git-pane" data-pane="log">
      {narrow ? null : rail}
      <div className="shui-git-column">
        {creating !== null ? (
          <div className="shui-git-form">
            <p className="shui-git-form-title">New worktree from {creating.label}</p>
            <NewWorktreeForm
              target={creating.label}
              busy={ops.busy}
              onCreate={(branch) => ops.create(branch, () => setCreating(null), creating.from)}
              onCancel={() => setCreating(null)}
            />
          </div>
        ) : null}
        {naming !== null ? (
          <div className="shui-git-form">
            <p className="shui-git-form-title">
              {naming.kind === 'branch' ? `New branch from ${naming.label}` : `Rename ${naming.label}`}
            </p>
            <BranchNameForm
              key={`${naming.kind}:${naming.from}`}
              initial={naming.kind === 'rename' ? naming.label : ''}
              label={naming.kind === 'branch' ? 'New branch name' : 'New branch name for the rename'}
              placeholder={
                naming.kind === 'branch'
                  ? `Branch (Enter to create from ${naming.label})`
                  : 'New name (Enter to rename)'
              }
              busy={ops.busy}
              onSubmit={(value) => {
                const done = (branch: string) => {
                  setNaming(null)
                  revealBranch(branch)
                }
                if (naming.kind === 'branch') ops.createBranch(value, naming.from, done)
                else ops.renameBranch(naming.from, value, done)
              }}
              onCancel={() => setNaming(null)}
            />
          </div>
        ) : null}
        {hint !== null ? <p className="shui-git-note-line warn">{hint}</p> : null}
        {merging !== null && target !== null ? (
          <div className="shui-git-form">
            <p className="shui-git-form-title">
              Merge {merging.branch} into {target}
            </p>
            <MergeForm
              draft={merging.draft}
              target={target}
              busy={ops.busy}
              worktree={merging.wt !== null}
              onChange={(draft) => setMerging((current) => (current === null ? null : { ...current, draft }))}
              onMerge={() => {
                const done = () => setMerging(null)
                if (merging.wt !== null) ops.merge(merging.wt, merging.draft, done)
                else ops.mergeBranch(merging.branch, merging.draft, done)
              }}
              onCancel={() => setMerging(null)}
            />
          </div>
        ) : null}
        {drill}
        <div
          className="shui-git-log"
          style={{
            gridTemplateColumns: narrow
              ? 'minmax(0, 1fr)'
              : `${layout.tree}px 5px minmax(240px, 1fr) 5px ${layout.details}px`,
          }}
        >
          {shows('branches') ? (
            <GitBranchTree
              nodes={nodes}
              defaultBranch={target}
              remotes={remotes}
              expanded={expanded}
              onExpanded={setExpanded}
              selected={treeSel}
              onSelect={selectNode}
              onAct={(node) => run(refActions(node), 'open')}
              onDelete={(node) => run(refActions(node), 'delete')}
              onMenu={(node, anchor) => menu.open(anchor, menuItems(refActions(node)))}
              onTap={
                narrow
                  ? (node, open) => {
                      if (node.kind === 'section' || node.kind === 'folder') setExpanded(node.id, !open)
                      else setStage('commits')
                    }
                  : undefined
              }
              onDrill={narrow ? () => setStage('commits') : undefined}
              onRename={(node) => run(refActions(node), 'rename')}
            />
          ) : null}
          {narrow ? null : (
            <Sash
              label="Resize the branches"
              width={layout.tree}
              side="left"
              onWidth={(tree) =>
                setStored((prev) => ({ ...normalize(prev), tree: Math.min(900, Math.max(160, tree)) }))
              }
            />
          )}
          {shows('commits') ? (
            <GitCommitList
              log={log}
              filter={filter}
              onFilter={setFilter}
              branchLabel={branchLabel}
              branches={branchChoices}
              onBranch={pickBranch}
              seeds={seeds}
              headColor={glyphColor(repoGlyph(snapshot?.refs.find((ref) => ref.current)?.name ?? null))}
              narrow={narrow}
              prefix={snapshot?.prefix ?? ''}
              labels={labels}
              remotes={remotes}
              glyph={repoGlyph}
              rings={rings}
              selected={commitSel}
              onSelect={(sha) => {
                setComparing(null)
                setCommitSel(sha)
              }}
              onAct={narrow ? () => setStage('commit') : openCommit}
              onMenu={(commit, anchor) => menu.open(anchor, menuItems(commitActions(commit)))}
              onTap={narrow ? () => setStage('commit') : undefined}
            />
          ) : null}
          {narrow ? null : (
            <Sash
              label="Resize the commit details"
              width={layout.details}
              side="right"
              onWidth={(width) =>
                setStored((prev) => ({ ...normalize(prev), details: Math.min(900, Math.max(160, width)) }))
              }
            />
          )}
          {shows('commit') && comparing !== null ? (
            <GitCompareFiles
              label={comparing.label}
              state={working}
              prefix={snapshot?.prefix ?? ''}
              top={here?.path ?? null}
              onOpen={(file) => onOpenCompareFile(file, comparing.ref)}
              onClose={() => setComparing(null)}
            />
          ) : shows('commit') ? (
            <GitCommitDetails
              state={details}
              selected={log.commits.length === 0 && !log.loading ? null : commitSel}
              shallow={snapshot?.shallow ?? false}
              onOpenFile={onOpenCommitFile}
              prefix={snapshot?.prefix ?? ''}
              top={here?.path ?? null}
              onSelectCommit={setCommitSel}
              view={layout.files}
              onView={(patch) =>
                setStored((prev) => {
                  const next = normalize(prev)
                  return { ...next, files: filesView({ ...next.files, ...patch }) }
                })
              }
              busy={ops.busy}
              onCompare={onOpenCompareFile}
              onEditSource={onOpenWorkingFile}
              onCommitFiles={ops.commitFiles}
              onCopyPatch={(sha, paths) => {
                setHint(null)
                gitCommitPatch(host, root, sha, paths)
                  .then((patch) => navigator.clipboard.writeText(patch))
                  .catch((err: unknown) => setHint(`copy as patch failed: ${errorMessage(err)}`))
              }}
              onHistory={(paths, sha) => {
                setFilter((prev) => ({ ...prev, paths, upTo: sha }))
                setCommitSel(sha)
                if (narrow) setStage('commits')
              }}
            />
          ) : null}
        </div>
      </div>
      {narrow ? rail : null}
      {menu.element}
      <ConfirmDialog
        open={pushing !== null}
        onOpenChange={(open) => {
          if (!open) setPushing(null)
        }}
        title={`Push ${pushing?.branch ?? ''}?`}
        description={
          pushing === null
            ? ''
            : pushing.target.setUpstream
              ? `It goes to ${pushing.target.name}, which becomes the branch it tracks.`
              : pushing.target.ahead === 0
                ? `${pushing.target.name} has its commits already.`
                : pushing.target.ahead === null
                  ? `Its commits go to ${pushing.target.name}.`
                  : `${pushing.target.ahead} ${pushing.target.ahead === 1 ? 'commit goes' : 'commits go'} to ${pushing.target.name}.`
        }
        confirmLabel="Push"
        onConfirm={() => {
          if (pushing !== null) ops.pushBranch(pushing.branch, pushing.target)
          setPushing(null)
        }}
        onCancel={() => setPushing(null)}
      />
    </div>
  )
}

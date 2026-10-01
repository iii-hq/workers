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
   branch actions in a bar along the bottom.

   The panes are memoized, and what this hands them keeps its identity
   until what it shows changed: a new selection, a details read or a form
   keystroke re-renders only the panes it concerns. */

import type { Host } from '@iii-dev/console-ui'
import { ConfirmDialog, EmptyState } from '@iii-dev/console-ui'
import { copyText, errorMessage } from '@iii-dev/console-ui/format'
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
import { type CSSProperties, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ActionRail, type GitAction, menuItems } from './ActionRail'
import { glyphColor, glyphOf } from './CommitGraph'
import { type ContextMenuAnchor, useContextMenu } from './ContextMenu'
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
  files: { height: null, grouped: true, info: true, preview: false },
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

export function filesView(raw: unknown): FilesView {
  const value = (raw ?? {}) as Partial<FilesView>
  return {
    height:
      typeof value.height === 'number' && Number.isFinite(value.height) ? Math.max(48, Math.round(value.height)) : null,
    grouped: value.grouped !== false,
    info: value.info !== false,
    preview: value.preview === true,
  }
}

function flatten(nodes: readonly RefTreeNode[], into = new Map<string, RefTreeNode>()): Map<string, RefTreeNode> {
  for (const node of nodes) {
    into.set(node.id, node)
    if (node.kind === 'section' || node.kind === 'folder') flatten(node.children, into)
  }
  return into
}

/** A drag handle between two panes; `side` says which pane it sizes. A
    drag sets the width in the grid's `variable` as it moves and keeps it
    once, on release: keeping it on every move re-rendered the whole Log. */
function Sash({
  label,
  width,
  side,
  variable,
  onWidth,
}: {
  label: string
  width: number
  side: 'left' | 'right'
  variable: string
  onWidth(next: number): void
}) {
  const dragged = useRef<number | null>(null)
  const drag = useSplitDrag<{ width: number; grid: HTMLElement | null }>({
    horizontal: true,
    begin: (event) => ({ width, grid: event.currentTarget.parentElement }),
    move: (origin, delta) => {
      const next = Math.min(900, Math.max(160, Math.round(origin.width + (side === 'left' ? delta : -delta))))
      dragged.current = next
      origin.grid?.style.setProperty(variable, `${next}px`)
    },
    step: (direction) => onWidth(width + (side === 'left' ? direction : -direction) * 16),
  })
  const end = () => {
    const next = dragged.current
    if (next === null) return
    dragged.current = null
    onWidth(next)
  }
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
      onPointerUp={(event) => {
        drag.onPointerUp(event)
        end()
      }}
      onPointerCancel={(event) => {
        drag.onPointerCancel(event)
        end()
      }}
      onLostPointerCapture={(event) => {
        drag.onLostPointerCapture(event)
        end()
      }}
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
  onOpenRevision,
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
  /** The file as commit `sha` left it, read-only. */
  onOpenRevision(file: CommitFile, sha: string): void
}) {
  const epoch = useWorktreeEpoch()
  const [filter, setFilter] = useState<LogFilter>({})
  const [treeSel, setTreeSel] = useState<string | null>(null)
  const [commitSel, setCommitSel] = useState<string | null>(null)
  const [stored, setStored] = usePaneState<Layout>(`iii::shell-ui::git::${paneKey}`, LAYOUT)
  const layout = useMemo(() => normalize(stored), [stored])
  // Kept by content: normalize() makes a new `expanded` array and `files`
  // object per stored value (a pane dragged wider), and a new open set
  // rebuilds every row of the tree.
  const expandedKey = layout.expanded.join('\n')
  const expanded = useMemo(() => new Set(expandedKey === '' ? [] : expandedKey.split('\n')), [expandedKey])
  const { height: filesHeight, grouped: filesGrouped, info: filesInfo, preview: filesPreview } = layout.files
  const files = useMemo(
    (): FilesView => ({ height: filesHeight, grouped: filesGrouped, info: filesInfo, preview: filesPreview }),
    [filesHeight, filesGrouped, filesInfo, filesPreview],
  )
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
  const openMenu = menu.open
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
  const snapshot = log.snapshot
  // Picking a branch selects its tip at once, from the refs: its page may
  // not be read yet. Picking it again leaves the commit picked alone.
  const selectTree = useCallback(
    (id: string | null) => {
      setTreeSel(id)
      if (id === treeSel) return
      const tip =
        id === 'head'
          ? (snapshot?.head ?? null)
          : id?.startsWith('ref:')
            ? (snapshot?.refs.find((ref) => ref.fullName === id.slice(4))?.sha ?? null)
            : null
      if (tip !== null) setCommitSel(tip)
    },
    [snapshot, treeSel],
  )
  // A branch picked in the tree or the Branch menu ends a "History up to here".
  const selectNode = useCallback(
    (id: string | null) => {
      setFilter((prev) => (prev.upTo === undefined ? prev : { ...prev, upTo: undefined }))
      selectTree(id)
    },
    [selectTree],
  )
  const details = useCommitDetails(host, root, snapshot, commitSel)
  const working = useWorkingDiff(host, root, snapshot?.prefix ?? null, comparing?.ref ?? null, epoch)

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

  const setExpanded = useCallback(
    (id: string, open: boolean) =>
      setStored((prev) => {
        const current = normalize(prev)
        const next = new Set(current.expanded)
        if (open) next.add(id)
        else next.delete(id)
        return { ...current, expanded: [...next] }
      }),
    [setStored],
  )

  // The selected commit (a branch picked selects its tip: selectTree). A
  // new list keeps the selection while it is listed, else takes the IDE's
  // HEAD, else the newest; a list that only grew keeps even an unlisted one
  // (a parent picked in the details).
  const head = snapshot?.head ?? null
  const tipSha =
    tipRef === null
      ? null
      : tipRef === 'HEAD'
        ? head
        : (snapshot?.refs.find((ref) => ref.fullName === tipRef)?.sha ?? null)
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
    selectTree(`ref:refs/heads/${name}`)
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
  // The latest push asked for: an earlier one's look-up answering late
  // must not name its branch.
  const pushSeq = useRef(0)
  const startPush = (branch: string) => {
    const list = ops.list
    if (list === null) return
    const seq = ++pushSeq.current
    setHint(null)
    pushTarget(host, list, branch).then(
      (target) => {
        if (seq === pushSeq.current) setPushing({ branch, target })
      },
      (err: unknown) => {
        if (seq === pushSeq.current) setHint(`push failed: ${errorMessage(err)}`)
      },
    )
  }
  const compareWith = (ref: string) => {
    setComparing({ ref, label: ref })
    if (narrow) setStage('commit')
  }

  const busyWhy = ops.busy ? 'another worktree operation is running' : null
  // Memoized on all it and its helpers (openRef, startPush, …) read.
  const refActions = useCallback(
    (node: RefTreeNode | null): GitAction[] => {
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
              : refActions(upNode).filter((action) =>
                  ['open', 'new-branch', 'new', 'diff', 'copy'].includes(action.id),
                ),
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
          run: () => void copyText(name),
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
    },
    [busyWhy, head, here?.path, host, narrow, nodeById, ops, remotes, setStored, snapshot, target],
  )
  const run = (actions: readonly GitAction[], id: string) => {
    const action = actions.find((candidate) => candidate.id === id)
    if (action && (action.blocked ?? null) === null) action.run()
  }

  const commitActions = useCallback(
    (commit: LogCommit): GitAction[] => {
      const refs = (labels.get(commit.sha) ?? []).filter((ref) => ref.kind !== 'tag').slice(0, 3)
      return [
        {
          id: 'copy-hash',
          group: 'copy',
          label: 'Copy hash',
          short: 'Copy',
          icon: <Copy aria-hidden />,
          run: () => void copyText(commit.sha),
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
    },
    [busyWhy, labels, narrow, ops, snapshot],
  )

  // Read when a commit is opened, so the list's handler keeps its identity
  // as the details arrive.
  const detailsRef = useRef(details.details)
  detailsRef.current = details.details
  const openCommit = useCallback(
    (commit: LogCommit) => {
      const current = detailsRef.current
      if (current !== null && current.sha === commit.sha) {
        const first = current.files[0]
        if (first !== undefined) onOpenCommitFile(first, current)
      } else {
        setCommitSel(commit.sha)
      }
    },
    [onOpenCommitFile],
  )

  const repoGlyph = useCallback((name: string | null) => glyphOf(name, target, remotes), [target, remotes])

  // The panes' handlers, one function each while what they read holds.
  const actOnRef = useCallback((node: RefTreeNode) => run(refActions(node), 'open'), [refActions])
  const deleteRef = useCallback((node: RefTreeNode) => run(refActions(node), 'delete'), [refActions])
  const renameRef = useCallback((node: RefTreeNode) => run(refActions(node), 'rename'), [refActions])
  const refMenu = useCallback(
    (node: RefTreeNode, anchor: ContextMenuAnchor) => openMenu(anchor, menuItems(refActions(node))),
    [openMenu, refActions],
  )
  const commitMenu = useCallback(
    (commit: LogCommit, anchor: ContextMenuAnchor) => openMenu(anchor, menuItems(commitActions(commit))),
    [openMenu, commitActions],
  )
  const moreMenu = useCallback(
    (anchor: ContextMenuAnchor, rest: GitAction[]) => openMenu(anchor, menuItems(rest, true)),
    [openMenu],
  )
  const showCommits = useCallback(() => setStage('commits'), [])
  const showCommit = useCallback(() => setStage('commit'), [])
  const tapBranch = useCallback(
    (node: RefTreeNode, open: boolean) => {
      if (node.kind === 'section' || node.kind === 'folder') setExpanded(node.id, !open)
      else setStage('commits')
    },
    [setExpanded],
  )
  const pickBranch = useCallback(
    (id: string | null) => {
      selectNode(id)
      for (let up = id === null ? undefined : parentOf.get(id); up !== undefined; up = parentOf.get(up)) {
        setExpanded(up, true)
      }
    },
    [selectNode, parentOf, setExpanded],
  )
  const selectCommit = useCallback((sha: string | null) => {
    setComparing(null)
    setCommitSel(sha)
  }, [])
  const setFilesView = useCallback(
    (patch: Partial<FilesView>) =>
      setStored((prev) => {
        const next = normalize(prev)
        return { ...next, files: filesView({ ...next.files, ...patch }) }
      }),
    [setStored],
  )
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
  const showHistory = useCallback(
    (paths: string[], sha: string) => {
      setFilter((prev) => ({ ...prev, paths, upTo: sha }))
      setCommitSel(sha)
      if (narrow) setStage('commits')
    },
    [narrow],
  )

  // With nothing picked in the tree, the rail acts on HEAD's branch (as
  // WebStorm's toolbar does), so it opens live rather than greyed out.
  const railNode = selectedNode ?? nodeById.get('head') ?? null
  // The narrow bar acts on what the stage shows: the commit on its own
  // stage (one Open, for its first branch), else the branch picked.
  const selectedCommit = commitSel === null ? null : (log.commits.find((commit) => commit.sha === commitSel) ?? null)
  const onCommit = narrow && stage === 'commit' && selectedCommit !== null
  const barCommit = onCommit ? selectedCommit : null
  const barActions = useMemo((): GitAction[] => {
    if (barCommit !== null) {
      const actions = commitActions(barCommit)
      const open = actions.find((action) => action.id.startsWith('open:'))
      return actions.filter((action) => !action.id.startsWith('open:') || action === open)
    }
    return refActions(railNode).filter((action) => !narrow || stage === 'branches' || action.id !== 'collapse')
  }, [barCommit, commitActions, refActions, railNode, narrow, stage])

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

  const rail = (
    <ActionRail
      label={onCommit ? 'Commit actions' : 'Branch actions'}
      actions={barActions}
      bar={narrow}
      onMore={moreMenu}
    />
  )
  const shows = (pane: Stage) => !narrow || stage === pane
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
          style={
            narrow
              ? { gridTemplateColumns: 'minmax(0, 1fr)' }
              : ({
                  '--git-tree': `${layout.tree}px`,
                  '--git-details': `${layout.details}px`,
                  gridTemplateColumns: 'var(--git-tree) 5px minmax(240px, 1fr) 5px var(--git-details)',
                } as CSSProperties)
          }
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
              onAct={actOnRef}
              onDelete={deleteRef}
              onMenu={refMenu}
              onTap={narrow ? tapBranch : undefined}
              onDrill={narrow ? showCommits : undefined}
              onRename={renameRef}
              narrow={narrow}
            />
          ) : null}
          {narrow ? null : (
            <Sash
              label="Resize the branches"
              width={layout.tree}
              side="left"
              variable="--git-tree"
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
              onSelect={selectCommit}
              onAct={narrow ? showCommit : openCommit}
              onMenu={commitMenu}
              onTap={narrow ? showCommit : undefined}
            />
          ) : null}
          {narrow ? null : (
            <Sash
              label="Resize the commit details"
              width={layout.details}
              side="right"
              variable="--git-details"
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
              onOpen={(file) => onOpenCompareFile(file, comparing.ref, file.from)}
              onClose={() => setComparing(null)}
            />
          ) : shows('commit') ? (
            <GitCommitDetails
              host={host}
              root={root}
              state={details}
              selected={log.commits.length === 0 && !log.loading ? null : commitSel}
              shallow={snapshot?.shallow ?? false}
              onOpenFile={onOpenCommitFile}
              prefix={snapshot?.prefix ?? ''}
              top={here?.path ?? null}
              onSelectCommit={setCommitSel}
              view={files}
              onView={setFilesView}
              busy={ops.busy}
              onCompare={onOpenCompareFile}
              onEditSource={onOpenWorkingFile}
              onCommitFiles={ops.commitFiles}
              onCopyPatch={copyPatch}
              onHistory={showHistory}
              onOpenRevision={onOpenRevision}
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

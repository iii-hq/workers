/* The Explorer view — @pierre/trees' FileTree over the page's flat tree.
   The tree renders into shadow DOM, so console classes can't reach it;
   theming rides the `--trees-*-override` CSS variables (custom properties
   inherit through shadow boundaries), mapped onto the console's design
   tokens.

   What this view owns, the way VS Code's explorer does:
   - single click previews a file, double click keeps it open;
   - folders list lazily: expanding one the snapshot did not reach asks
     the page for its children;
   - a right-click menu per file, folder and on the empty space;
   - inline create and rename rows (F2), delete with confirmation;
   - Git letters beside changed files, a dot on folders that hold one.

   The FlatTree in props is the source of truth; the model is patched by
   diff so expansion, focus and selection survive a watcher burst. */

import { ConfirmDialog, IconButton } from '@iii-dev/console-ui'
import type { FileTreeDirectoryHandle, FileTreeRowDecoration, GitStatusEntry } from '@pierre/trees'
import { FileTree, useFileTree } from '@pierre/trees/react'
import {
  ChevronsDownUp,
  ClipboardCopy,
  Copy,
  FileDiff,
  FilePlus,
  FolderPlus,
  Pencil,
  RefreshCw,
  Search,
  SquareTerminal,
  TextSearch,
  Trash2,
  Undo2,
  X,
} from 'lucide-react'
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import type { FlatTree } from './coder'
import { anchorFromEvent, type ContextMenuItem, useContextMenu } from './ContextMenu'
import type { GitFileStatus } from './git'
import { statusLetter, statusTitle } from './git-actions'
import { ancestorDirs, basename, dirname, joinRel, stripDirSlash } from './paths'
import {
  reactivateSelectedFile,
  shouldActivateTreeSelection,
  treeItemFromEvent,
} from './tree-activation'
import { expandedDirectoryPaths } from './tree-expansion'
import { visibleTruncations } from './tree-model'
import { TREE_THEME, TREE_UNSAFE_CSS } from './tree-theme'
import { ViewHeader } from './ViewHeader'

const EXPAND_SNAPSHOT_MS = 250
/** Above this many path additions/removals the model is rebuilt instead of patched. */
const BATCH_PATCH_LIMIT = 400
const CREATE_PLACEHOLDER = 'untitled'

/** Everything the explorer can do to the workspace. All paths are
    root-relative and slash-less; every verb may reject with a message the
    view shows in place. */
export interface ExplorerActions {
  create: (kind: 'file' | 'folder', rel: string) => Promise<void>
  rename: (from: string, to: string, isDir: boolean) => Promise<void>
  remove: (rel: string, isDir: boolean) => Promise<void>
  duplicate: (rel: string) => Promise<void>
  openTerminal: (dir: string) => void
  copyPath: (rel: string, absolute: boolean) => void
  compare: (rel: string) => void
  /** Search the folder's contents in the Search view. */
  findInFolder: (dir: string) => void
  discard: (rel: string) => void
  refresh: () => void
}

interface FilesTabProps {
  tree: FlatTree | null
  gitStatus: readonly GitStatusEntry[]
  theme: 'light' | 'dark'
  /** True while dot entries are filtered out of the listing — an empty
      tree then reads as filtered, not as an empty folder. */
  hiddenFiltered: boolean
  rootLabel: string
  /** Slash-less dir paths to expand when (re)setting the tree. */
  expanded: readonly string[]
  /** Debounced snapshot of the currently expanded dirs (slash-less). */
  onExpandedChange: (paths: string[]) => void
  /** A folder just opened; the page lists it if the snapshot did not. */
  onExpandDir: (dir: string) => void
  loadingDirs: ReadonlySet<string>
  /** Slash-less path to expand + scroll to (a search "folders" result or
      "reveal in explorer"); acknowledged through onRevealed. */
  reveal: string | null
  onRevealed: () => void
  /** The file currently shown in the editor or diff. */
  activePath: string | null
  /** Single click on a file — review a change or preview a clean file. */
  onActivateFile: (relPath: string) => void
  /** Double click on a file — pin clean files; keep changes in review. */
  onPinFile: (relPath: string) => void
  actions: ExplorerActions
}

interface PendingDelete {
  path: string
  isDir: boolean
}

export function FilesTab({
  tree,
  gitStatus,
  theme,
  hiddenFiltered,
  rootLabel,
  expanded,
  onExpandedChange,
  onExpandDir,
  loadingDirs,
  reveal,
  onRevealed,
  activePath,
  onActivateFile,
  onPinFile,
  actions,
}: FilesTabProps) {
  const filterId = useId()
  const [filter, setFilter] = useState('')
  const [filterMatchCount, setFilterMatchCount] = useState<number | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [pendingDelete, setPendingDelete] = useState<PendingDelete | null>(null)
  const menu = useContextMenu()
  // The model is created once per component lifetime; data arriving
  // later flows through batch/resetPaths/setGitStatus below. Selection
  // opens through a ref so the creation-time callback never goes stale.
  const openRef = useRef<(paths: readonly string[]) => void>(() => {})
  const activePathRef = useRef(activePath)
  activePathRef.current = activePath
  const lastFileRef = useRef<string | null>(null)
  const skipClickPathRef = useRef<string | null>(null)
  const actionsRef = useRef(actions)
  actionsRef.current = actions
  const statusRef = useRef<ReadonlyMap<string, GitFileStatus>>(new Map())
  const changedDirsRef = useRef<ReadonlySet<string>>(new Set())
  // Placeholders of inline creates, keyed by the model path they occupy.
  const placeholdersRef = useRef(new Map<string, 'file' | 'folder'>())
  const treeRef = useRef(tree)
  treeRef.current = tree

  const { model } = useFileTree({
    fileTreeSearchMode: 'hide-non-matches',
    flattenEmptyDirectories: false,
    icons: { set: 'complete', colored: true },
    itemHeight: 24,
    paths: tree?.paths ?? [],
    search: false,
    stickyFolders: true,
    unsafeCSS: TREE_UNSAFE_CSS,
    onSelectionChange: (selected) => openRef.current(selected),
    renderRowDecoration: ({ item }): FileTreeRowDecoration | null => {
      const key = stripDirSlash(item.path)
      if (item.kind === 'file') {
        const status = statusRef.current.get(key)
        return status ? { text: statusLetter(status), title: statusTitle(status) } : null
      }
      return changedDirsRef.current.has(key) ? { text: '•', title: 'Contains changes' } : null
    },
    renaming: {
      canRename: () => true,
      onError: (error) => setNote(error),
      onRename: ({ sourcePath, destinationPath, isFolder }) => {
        const from = stripDirSlash(sourcePath)
        const to = stripDirSlash(destinationPath)
        const placeholder = placeholdersRef.current.get(sourcePath)
        if (placeholder !== undefined) {
          placeholdersRef.current.delete(sourcePath)
          void actionsRef.current.create(placeholder, to).catch((error: unknown) => {
            setNote(error instanceof Error ? error.message : String(error))
            try {
              model.remove(destinationPath, { recursive: true })
            } catch {
              // The path may already be gone from the model.
            }
          })
          return
        }
        if (from === to) return
        void actionsRef.current.rename(from, to, isFolder).catch((error: unknown) => {
          setNote(error instanceof Error ? error.message : String(error))
          try {
            model.move(destinationPath, sourcePath)
          } catch {
            // The next tree diff resets the row.
          }
        })
      },
    },
  })

  const kinds = tree?.kinds
  useEffect(() => {
    openRef.current = (selected) => {
      const path = selected[0]
      if (!path || !kinds) return
      if (kinds.get(path) === 'file') {
        lastFileRef.current = path
        // Controlled selection mirrors a file already opened by live
        // follow. It must not re-enter activation and cancel that diff.
        if (!shouldActivateTreeSelection(path, activePathRef.current)) return
        // @pierre/trees only reports selection CHANGES. Remember this
        // activation for the bubbling click so a new selection does not
        // open twice; a later click on the same row still reactivates it.
        skipClickPathRef.current = path
        queueMicrotask(() => {
          if (skipClickPathRef.current === path) skipClickPathRef.current = null
        })
        onActivateFile(path)
      } else {
        // A dir selection must not leave a stale file behind — the
        // wrapper's dblclick pin would hit the wrong path.
        lastFileRef.current = null
      }
    }
  }, [kinds, onActivateFile])

  // Expansion state is read back through the dir handles (the model has
  // no expansion events on its public surface): every model notification
  // schedules a debounced snapshot over the known dir paths, and a folder
  // that just opened is reported at once so its listing can start.
  const liveExpandedRef = useRef<readonly string[]>(expanded)
  useEffect(() => {
    liveExpandedRef.current = expanded
  }, [expanded])
  const lastReportedRef = useRef<string>('')
  const openDirsRef = useRef<Set<string>>(new Set(expanded))
  useEffect(() => {
    if (!kinds) return
    const dirPaths: string[] = []
    for (const [path, kind] of kinds) {
      if (kind === 'dir') dirPaths.push(path)
    }
    let timer: number | null = null
    const snapshot = () => {
      timer = null
      const open = expandedDirectoryPaths(model, dirPaths)
      liveExpandedRef.current = open
      const key = open.join('\n')
      if (key === lastReportedRef.current) return
      lastReportedRef.current = key
      onExpandedChange(open)
    }
    const unsubscribe = model.subscribe(() => {
      const open = expandedDirectoryPaths(model, dirPaths)
      liveExpandedRef.current = open
      const known = openDirsRef.current
      for (const dir of open) {
        if (!known.has(dir)) onExpandDir(dir)
      }
      openDirsRef.current = new Set(open)
      if (timer != null) window.clearTimeout(timer)
      timer = window.setTimeout(snapshot, EXPAND_SNAPSHOT_MS)
    })
    return () => {
      if (timer != null) window.clearTimeout(timer)
      unsubscribe()
    }
  }, [model, kinds, onExpandedChange, onExpandDir])

  // Keep the model in step with the flat tree by diff: small bursts are
  // patched in place (expansion, focus and selection survive), a root
  // change or a large delta rebuilds.
  const lastPathsRef = useRef<Set<string> | null>(null)
  useEffect(() => {
    if (!tree) {
      lastPathsRef.current = null
      return
    }
    const next = new Set(tree.paths)
    const prev = lastPathsRef.current
    lastPathsRef.current = next
    const reset = () => {
      // The model accepts dir ids in either spelling depending on how the
      // row materialized — hand it both.
      const initialExpandedPaths = liveExpandedRef.current.flatMap((p) => [p, `${p}/`])
      model.resetPaths(tree.paths, { initialExpandedPaths })
    }
    if (prev === null) {
      reset()
      return
    }
    const adds: string[] = []
    for (const p of next) if (!prev.has(p)) adds.push(p)
    const removes: string[] = []
    for (const p of prev) if (!next.has(p)) removes.push(p)
    if (adds.length === 0 && removes.length === 0) return
    if (adds.length + removes.length > BATCH_PATCH_LIMIT) {
      reset()
      return
    }
    const removeSet = new Set(removes.map(stripDirSlash))
    const topRemoves = removes.filter(
      (p) => !ancestorDirs(stripDirSlash(p)).some((ancestor) => removeSet.has(ancestor)),
    )
    adds.sort((a, b) => a.length - b.length)
    try {
      model.batch([
        ...topRemoves
          .filter((p) => model.getItem(p) !== null)
          .map((p) => ({ type: 'remove' as const, path: p, recursive: true })),
        ...adds.filter((p) => model.getItem(p) === null).map((p) => ({ type: 'add' as const, path: p })),
      ])
    } catch {
      reset()
    }
  }, [model, tree])

  // Expansion reports normally originate in the model itself. Changed
  // ancestors are also added by the page, so explicitly open any newly
  // requested handles without collapsing folders the user closed.
  // biome-ignore lint/correctness/useExhaustiveDependencies: a fresh tree materializes rows the handles need
  useEffect(() => {
    for (const path of expanded) {
      const handle = model.getItem(path) ?? model.getItem(`${path}/`)
      if (handle?.isDirectory()) (handle as FileTreeDirectoryHandle).expand()
    }
  }, [model, expanded, tree])

  // Keep the tree's selection aligned with the file currently visible in
  // the main pane, including files opened by live follow or the tabs.
  // biome-ignore lint/correctness/useExhaustiveDependencies: a fresh tree can hold the row to select
  useEffect(() => {
    const selected = model.getSelectedPaths()
    if (activePath === null || kinds?.get(activePath) !== 'file') {
      for (const path of selected) model.getItem(path)?.deselect()
      return
    }
    if (selected.length === 1 && selected[0] === activePath) return
    for (const path of selected) model.getItem(path)?.deselect()
    model.getItem(activePath)?.select()
  }, [model, activePath, kinds, tree])

  // The filter lives outside the tree's shadow DOM and drives the model
  // directly, avoiding a second built-in search surface.
  // biome-ignore lint/correctness/useExhaustiveDependencies: match counts change with the tree
  useEffect(() => {
    const query = filter.trim()
    model.setSearch(query === '' ? null : query)
    setFilterMatchCount(query === '' ? null : model.getSearchMatchingPaths().length)
  }, [model, filter, tree])

  useEffect(() => {
    const byPath = new Map<string, GitFileStatus>()
    const dirs = new Set<string>()
    for (const entry of gitStatus) {
      byPath.set(entry.path, entry.status)
      for (const dir of ancestorDirs(entry.path)) dirs.add(dir)
    }
    statusRef.current = byPath
    changedDirsRef.current = dirs
    model.setGitStatus(gitStatus.length > 0 ? gitStatus : undefined)
  }, [model, gitStatus])

  // Reveal a path: expand every ancestor handle, scroll it into view,
  // then acknowledge. Expansion only ever OPENS dirs, so this can't fight
  // the debounced expansion snapshot. A folder still being listed waits
  // for the tree to catch up (the effect re-runs on `tree`).
  // biome-ignore lint/correctness/useExhaustiveDependencies: the target row may only exist after a fetch
  useEffect(() => {
    if (!reveal || !kinds) return
    const target = model.getItem(reveal) ?? model.getItem(`${reveal}/`)
    if (target === null) {
      onExpandDir(reveal)
      return
    }
    for (const ancestor of ancestorDirs(reveal)) {
      const handle = model.getItem(ancestor) ?? model.getItem(`${ancestor}/`)
      if (handle?.isDirectory()) (handle as FileTreeDirectoryHandle).expand()
    }
    if (target.isDirectory()) (target as FileTreeDirectoryHandle).expand()
    model.scrollToPath(target.getPath(), { offset: 'center', focus: true })
    onRevealed()
  }, [reveal, kinds, model, onRevealed, onExpandDir, tree])

  // ── verbs ──
  const beginCreate = useCallback(
    (kind: 'file' | 'folder', dir: string) => {
      setNote(null)
      setFilter('')
      if (dir !== '') {
        const handle = model.getItem(dir) ?? model.getItem(`${dir}/`)
        if (handle?.isDirectory()) (handle as FileTreeDirectoryHandle).expand()
      }
      let name = CREATE_PLACEHOLDER
      let n = 2
      while (model.getItem(joinRel(dir, name)) !== null || model.getItem(`${joinRel(dir, name)}/`) !== null) {
        name = `${CREATE_PLACEHOLDER}-${n}`
        n += 1
      }
      const rel = joinRel(dir, name)
      const modelPath = kind === 'folder' ? `${rel}/` : rel
      try {
        model.add(modelPath)
      } catch (error) {
        setNote(error instanceof Error ? error.message : String(error))
        return
      }
      placeholdersRef.current.set(modelPath, kind)
      // The row mounts on the next frame; renaming needs it in the DOM.
      window.requestAnimationFrame(() => {
        if (!model.startRenaming(modelPath, { removeIfCanceled: true })) {
          placeholdersRef.current.delete(modelPath)
          try {
            model.remove(modelPath, { recursive: true })
          } catch {
            // nothing to undo
          }
        }
      })
    },
    [model],
  )

  const beginRename = useCallback(
    (path: string) => {
      setNote(null)
      const handle = model.getItem(path) ?? model.getItem(`${path}/`)
      if (handle === null) return
      if (!model.startRenaming(handle.getPath())) setNote(`cannot rename ${basename(path)}`)
    },
    [model],
  )

  const collapseAll = useCallback(() => {
    if (!kinds) return
    for (const [path, kind] of kinds) {
      if (kind !== 'dir') continue
      const handle = model.getItem(`${path}/`) ?? model.getItem(path)
      if (handle?.isDirectory()) (handle as FileTreeDirectoryHandle).collapse()
    }
  }, [model, kinds])

  const confirmDelete = useCallback(() => {
    const target = pendingDelete
    setPendingDelete(null)
    if (!target) return
    void actionsRef.current.remove(target.path, target.isDir).catch((error: unknown) => {
      setNote(error instanceof Error ? error.message : String(error))
    })
  }, [pendingDelete])

  const itemsForFile = useCallback(
    (rel: string): ContextMenuItem[] => {
      const changed = statusRef.current.has(rel)
      return [
        { id: 'open', label: 'Open', icon: <TextSearch />, onSelect: () => onPinFile(rel) },
        { id: 'compare', label: 'Compare with…', icon: <FileDiff />, onSelect: () => actionsRef.current.compare(rel) },
        ...(changed
          ? [
              {
                id: 'discard',
                label: 'Discard changes',
                icon: <Undo2 />,
                danger: true,
                onSelect: () => actionsRef.current.discard(rel),
              } satisfies ContextMenuItem,
            ]
          : []),
        { type: 'separator', id: 's1' },
        { id: 'copy-path', label: 'Copy path', icon: <ClipboardCopy />, onSelect: () => actionsRef.current.copyPath(rel, true) },
        {
          id: 'copy-rel',
          label: 'Copy relative path',
          icon: <ClipboardCopy />,
          onSelect: () => actionsRef.current.copyPath(rel, false),
        },
        { type: 'separator', id: 's2' },
        { id: 'rename', label: 'Rename…', icon: <Pencil />, shortcut: 'F2', onSelect: () => beginRename(rel) },
        {
          id: 'duplicate',
          label: 'Duplicate',
          icon: <Copy />,
          onSelect: () =>
            void actionsRef.current.duplicate(rel).catch((error: unknown) => {
              setNote(error instanceof Error ? error.message : String(error))
            }),
        },
        {
          id: 'delete',
          label: 'Delete…',
          icon: <Trash2 />,
          shortcut: '⌫',
          danger: true,
          onSelect: () => setPendingDelete({ path: rel, isDir: false }),
        },
      ]
    },
    [onPinFile, beginRename],
  )

  const itemsForDir = useCallback(
    (dir: string): ContextMenuItem[] => [
      { id: 'new-file', label: 'New file…', icon: <FilePlus />, onSelect: () => beginCreate('file', dir) },
      { id: 'new-folder', label: 'New folder…', icon: <FolderPlus />, onSelect: () => beginCreate('folder', dir) },
      { type: 'separator', id: 's1' },
      { id: 'terminal', label: 'Open in terminal', icon: <SquareTerminal />, onSelect: () => actionsRef.current.openTerminal(dir) },
      { id: 'find', label: 'Find in folder…', icon: <Search />, onSelect: () => actionsRef.current.findInFolder(dir) },
      { type: 'separator', id: 's2' },
      { id: 'copy-path', label: 'Copy path', icon: <ClipboardCopy />, onSelect: () => actionsRef.current.copyPath(dir, true) },
      { id: 'copy-rel', label: 'Copy relative path', icon: <ClipboardCopy />, onSelect: () => actionsRef.current.copyPath(dir, false) },
      { type: 'separator', id: 's3' },
      { id: 'rename', label: 'Rename…', icon: <Pencil />, shortcut: 'F2', onSelect: () => beginRename(dir) },
      {
        id: 'delete',
        label: 'Delete…',
        icon: <Trash2 />,
        shortcut: '⌫',
        danger: true,
        onSelect: () => setPendingDelete({ path: dir, isDir: true }),
      },
    ],
    [beginCreate, beginRename],
  )

  const itemsForRoot = useCallback(
    (): ContextMenuItem[] => [
      { id: 'new-file', label: 'New file…', icon: <FilePlus />, onSelect: () => beginCreate('file', '') },
      { id: 'new-folder', label: 'New folder…', icon: <FolderPlus />, onSelect: () => beginCreate('folder', '') },
      { type: 'separator', id: 's1' },
      { id: 'terminal', label: 'Open in terminal', icon: <SquareTerminal />, onSelect: () => actionsRef.current.openTerminal('') },
      { id: 'refresh', label: 'Refresh', icon: <RefreshCw />, onSelect: () => actionsRef.current.refresh() },
      { id: 'collapse', label: 'Collapse folders', icon: <ChevronsDownUp />, onSelect: collapseAll },
    ],
    [beginCreate, collapseAll],
  )

  const openMenuAt = useCallback(
    (anchor: { x: number; y: number }, item: { path: string; kind: 'file' | 'directory' } | null) => {
      if (item === null) {
        menu.open(anchor, itemsForRoot())
        return
      }
      const rel = stripDirSlash(item.path)
      // Focus follows the right-click, as in every file manager.
      model.getItem(item.path)?.focus()
      menu.open(anchor, item.kind === 'directory' ? itemsForDir(rel) : itemsForFile(rel))
    },
    [menu, model, itemsForDir, itemsForFile, itemsForRoot],
  )

  const onTreeKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLElement>) => {
      const focused = model.getFocusedPath()
      if (event.key === 'F2' && focused) {
        event.preventDefault()
        beginRename(stripDirSlash(focused))
        return
      }
      if ((event.key === 'Delete' || (event.key === 'Backspace' && (event.metaKey || event.ctrlKey))) && focused) {
        event.preventDefault()
        setPendingDelete({ path: stripDirSlash(focused), isDir: focused.endsWith('/') })
        return
      }
      if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && focused) {
        event.preventDefault()
        openMenuAt(anchorFromEvent(event), { path: focused, kind: focused.endsWith('/') ? 'directory' : 'file' })
      }
    },
    [model, beginRename, openMenuAt],
  )

  const truncations = useMemo(() => (tree ? visibleTruncations(tree.truncations) : []), [tree])
  const pendingDeleteName = pendingDelete ? basename(pendingDelete.path) : ''

  return (
    <div className="shui-tree-wrap">
      <ViewHeader
        title="Explorer"
        detail={<span className="shui-view-root" title={rootLabel}>{rootLabel}</span>}
        actions={
          <>
            <IconButton label="New file" onClick={() => beginCreate('file', dirOfFocus(model))}>
              <FilePlus aria-hidden />
            </IconButton>
            <IconButton label="New folder" onClick={() => beginCreate('folder', dirOfFocus(model))}>
              <FolderPlus aria-hidden />
            </IconButton>
            <IconButton label="Refresh explorer" onClick={() => actions.refresh()}>
              <RefreshCw aria-hidden />
            </IconButton>
            <IconButton label="Collapse folders" onClick={collapseAll}>
              <ChevronsDownUp aria-hidden />
            </IconButton>
          </>
        }
      />
      <div className="shui-tree-filter-shell">
        <div className="shui-tree-filter">
          <label className="shui-sr-only" htmlFor={filterId}>
            Filter files
          </label>
          <Search aria-hidden className="shui-tree-filter-search" />
          <input
            id={filterId}
            type="text"
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder="Filter files…"
            autoComplete="off"
            spellCheck={false}
            onKeyDown={(event) => {
              if (event.key === 'Escape' && filter !== '') {
                event.preventDefault()
                setFilter('')
              }
            }}
          />
          {filter !== '' ? (
            <button
              type="button"
              className="shui-tree-filter-clear"
              aria-label="Clear file filter"
              title="clear file filter"
              onClick={() => setFilter('')}
            >
              <X aria-hidden />
            </button>
          ) : null}
        </div>
      </div>
      {note !== null ? (
        <div className="shui-tree-note warn" role="alert">
          <span>{note}</span>
          <button type="button" className="shui-tree-note-close" aria-label="Dismiss" onClick={() => setNote(null)}>
            <X aria-hidden />
          </button>
        </div>
      ) : null}

      {/* biome-ignore lint/a11y/noStaticElementInteractions: the stage relays the empty-space context menu */}
      <div
        className="shui-tree-stage"
        onContextMenu={(event) => {
          event.preventDefault()
          openMenuAt(anchorFromEvent(event), treeItemFromEvent(event))
        }}
      >
        {!tree ? (
          <div className="shui-side-note">loading tree…</div>
        ) : tree.paths.length === 0 ? (
          <div className="shui-side-note">
            {hiddenFiltered ? 'nothing visible — hidden entries are filtered' : 'empty folder'}
          </div>
        ) : filterMatchCount === 0 ? (
          <div className="shui-side-note">no matching files</div>
        ) : (
          <FileTree
            model={model}
            className="shui-tree"
            style={{ ...TREE_THEME, colorScheme: theme }}
            data-autofocus=""
            onKeyDown={onTreeKeyDown}
            onClick={(event) => {
              const selectedPath = model.getSelectedPaths()[0] ?? null
              if (skipClickPathRef.current === selectedPath) {
                skipClickPathRef.current = null
                return
              }
              reactivateSelectedFile(event, selectedPath, onActivateFile)
            }}
            onDoubleClick={() => {
              if (lastFileRef.current) onPinFile(lastFileRef.current)
            }}
          />
        )}
      </div>

      {loadingDirs.size > 0 ? (
        <div className="shui-side-note ghost" role="status">
          listing {loadingDirs.size === 1 ? basename([...loadingDirs][0] || rootLabel) : `${loadingDirs.size} folders`}…
        </div>
      ) : truncations.length > 0 ? (
        <div className="shui-side-note ghost">
          partial listing — {truncations.length} {truncations.length === 1 ? 'folder' : 'folders'} capped;
          expand a folder to list it
        </div>
      ) : null}

      {menu.element}
      <ConfirmDialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null)
        }}
        title={pendingDelete?.isDir ? `Delete folder ${pendingDeleteName}?` : `Delete ${pendingDeleteName}?`}
        description={
          pendingDelete?.isDir
            ? 'The folder and everything inside it are removed from disk.'
            : 'The file is removed from disk.'
        }
        details={pendingDelete ? [pendingDelete.path] : undefined}
        confirmLabel="Delete"
        onConfirm={confirmDelete}
        onCancel={() => setPendingDelete(null)}
      />
    </div>
  )
}

/** The folder a header-bar "new file" lands in: the focused folder, the
    focused file's folder, or the root. */
function dirOfFocus(model: { getFocusedPath(): string | null }): string {
  const focused = model.getFocusedPath()
  if (!focused) return ''
  return focused.endsWith('/') ? stripDirSlash(focused) : dirname(focused)
}

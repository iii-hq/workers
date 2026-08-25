/* The files tab — @pierre/trees' FileTree over the worker's
   `coder::tree` listing. The tree renders into shadow DOM, so console
   classes can't reach it; theming rides the `--trees-*-override` CSS
   variables (custom properties inherit through shadow boundaries),
   mapped onto the console's design tokens.

   Open semantics: single click activates a file (changed files review,
   clean files preview), double click delegates pin/review behavior to
   the page. The dblclick is a composed DOM event, so it bubbles out of
   the tree's shadow root to the wrapper. Folder expansion is
   reported (debounced) for per-tab persistence and replayed through
   `resetPaths({ initialExpandedPaths })`. */

import type { FileTreeDirectoryHandle, GitStatusEntry } from '@pierre/trees'
import { FileTree, useFileTree } from '@pierre/trees/react'
import { FilePlus, FolderPlus, Search, X } from 'lucide-react'
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import type { FlatTree } from './coder'
import {
  reactivateSelectedFile,
  shouldActivateTreeSelection,
} from './tree-activation'
import { TREE_THEME, TREE_UNSAFE_CSS } from './tree-theme'

/** Directory paths live in the model with a trailing slash; callers key
    everything slash-less — accept either on the way in, strip on the
    way out. */
const EXPAND_SNAPSHOT_MS = 250

interface FilesTabProps {
  tree: FlatTree | null
  gitStatus: readonly GitStatusEntry[]
  theme: 'light' | 'dark'
  /** True while dot entries are filtered out of the listing — an empty
      tree then reads as filtered, not as an empty folder. */
  hiddenFiltered: boolean
  /** Slash-less dir paths to expand when (re)setting the tree. */
  expanded: readonly string[]
  /** Debounced snapshot of the currently expanded dirs (slash-less). */
  onExpandedChange: (paths: string[]) => void
  /** Slash-less dir path to expand + scroll to (a search "folders"
      result); acknowledged through onRevealed. */
  reveal: string | null
  onRevealed: () => void
  /** The file currently shown in the editor or diff. */
  activePath: string | null
  /** Single click on a file — review a change or preview a clean file. */
  onActivateFile: (relPath: string) => void
  /** Double click on a file — pin clean files; keep changes in review. */
  onPinFile: (relPath: string) => void
  /** Create a file or folder at a root-relative path; parents are made. */
  onCreate?: (kind: 'file' | 'folder', rel: string) => Promise<void>
}

export function FilesTab({
  tree,
  gitStatus,
  theme,
  hiddenFiltered,
  expanded,
  onExpandedChange,
  reveal,
  onRevealed,
  activePath,
  onActivateFile,
  onPinFile,
  onCreate,
}: FilesTabProps) {
  const filterId = useId()
  const [filter, setFilter] = useState('')
  const [createKind, setCreateKind] = useState<'file' | 'folder' | null>(null)
  const [createName, setCreateName] = useState('')
  const [createError, setCreateError] = useState<string | null>(null)
  const submitCreate = async () => {
    const rel = createName.trim().replace(/^\/+/, '')
    if (!rel || !createKind || !onCreate) return
    // Paths stay inside the browsed root: a "." or ".." segment is either
    // noise or an escape attempt, and the jail would reject it anyway.
    if (rel.split('/').some((part) => part === '.' || part === '..')) {
      setCreateError('the path cannot contain "." or ".." segments')
      return
    }
    try {
      await onCreate(createKind, rel)
      setCreateKind(null)
      setCreateName('')
      setCreateError(null)
    } catch (error) {
      setCreateError(error instanceof Error ? error.message : String(error))
    }
  }
  const [filterMatchCount, setFilterMatchCount] = useState<number | null>(null)
  // The model is created once per component lifetime; data arriving
  // later flows through resetPaths/setGitStatus below. Selection opens
  // through a ref so the creation-time callback never goes stale.
  const openRef = useRef<(paths: readonly string[]) => void>(() => {})
  const activePathRef = useRef(activePath)
  activePathRef.current = activePath
  const lastFileRef = useRef<string | null>(null)
  const skipClickPathRef = useRef<string | null>(null)
  const { model } = useFileTree({
    fileTreeSearchMode: 'hide-non-matches',
    flattenEmptyDirectories: true,
    icons: { set: 'complete', colored: true },
    itemHeight: 29,
    paths: tree?.paths ?? [],
    search: false,
    stickyFolders: true,
    unsafeCSS: TREE_UNSAFE_CSS,
    onSelectionChange: (selected) => openRef.current(selected),
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
  // schedules a debounced snapshot over the known dir paths.
  const expandedRef = useRef<readonly string[]>(expanded)
  expandedRef.current = expanded
  const lastReportedRef = useRef<string>('')
  useEffect(() => {
    if (!kinds) return
    const dirPaths: string[] = []
    for (const [path, kind] of kinds) {
      if (kind === 'dir') dirPaths.push(path)
    }
    let timer: number | null = null
    const snapshot = () => {
      timer = null
      const open: string[] = []
      for (const path of dirPaths) {
        const handle = model.getItem(path) ?? model.getItem(`${path}/`)
        // A method call doesn't narrow the handle union — the literal
        // `isDirectory(): true` return type needs the explicit cast.
        if (
          handle?.isDirectory() &&
          (handle as FileTreeDirectoryHandle).isExpanded()
        ) {
          open.push(path)
        }
      }
      const key = open.join('\n')
      if (key === lastReportedRef.current) return
      lastReportedRef.current = key
      onExpandedChange(open)
    }
    const unsubscribe = model.subscribe(() => {
      if (timer != null) window.clearTimeout(timer)
      timer = window.setTimeout(snapshot, EXPAND_SNAPSHOT_MS)
    })
    return () => {
      if (timer != null) window.clearTimeout(timer)
      unsubscribe()
    }
  }, [model, kinds, onExpandedChange])

  // `useFileTree` ignores option changes after creation by design —
  // push updates through the model's explicit methods. Expansion rides
  // each reset (the store drops it otherwise); the ref keeps this
  // effect from re-running on every expand/collapse report.
  const pathsKey = useMemo(() => tree?.paths.join('\n') ?? '', [tree])
  const lastPathsKeyRef = useRef('')
  useEffect(() => {
    if (pathsKey === lastPathsKeyRef.current) return
    lastPathsKeyRef.current = pathsKey
    // The model accepts dir ids in either spelling depending on how the
    // row materialized — hand it both.
    const initialExpandedPaths = expandedRef.current.flatMap((p) => [
      p,
      `${p}/`,
    ])
    model.resetPaths(tree?.paths ?? [], { initialExpandedPaths })
  }, [model, pathsKey, tree])

  // Expansion reports normally originate in the model itself. Changed
  // ancestors are also added by the page, so explicitly open any newly
  // requested handles without collapsing folders the user closed.
  useEffect(() => {
    for (const path of expanded) {
      const handle = model.getItem(path) ?? model.getItem(`${path}/`)
      if (handle?.isDirectory()) (handle as FileTreeDirectoryHandle).expand()
    }
  }, [model, pathsKey, expanded])

  // Keep the Files tree's selection aligned with the file currently
  // visible in the main pane, including files opened by live follow.
  useEffect(() => {
    const selected = model.getSelectedPaths()
    if (activePath === null || kinds?.get(activePath) !== 'file') {
      for (const path of selected) model.getItem(path)?.deselect()
      return
    }
    if (selected.length === 1 && selected[0] === activePath) return
    for (const path of selected) model.getItem(path)?.deselect()
    model.getItem(activePath)?.select()
  }, [model, activePath, kinds, pathsKey])

  // Codex keeps the filter outside the tree's shadow DOM and drives the
  // model directly, avoiding a second built-in search surface.
  useEffect(() => {
    const query = filter.trim()
    model.setSearch(query === '' ? null : query)
    setFilterMatchCount(
      query === '' ? null : model.getSearchMatchingPaths().length,
    )
  }, [model, filter, pathsKey])

  useEffect(() => {
    model.setGitStatus(gitStatus.length > 0 ? gitStatus : undefined)
  }, [model, gitStatus])

  // Reveal a folder from a search result: expand every ancestor handle,
  // scroll it into view, then acknowledge. Expansion only ever OPENS
  // dirs, so this can't fight the debounced expansion snapshot.
  useEffect(() => {
    if (!reveal || !kinds) return
    const segs = reveal.split('/')
    for (let i = 1; i <= segs.length; i++) {
      const p = segs.slice(0, i).join('/')
      const handle = model.getItem(p) ?? model.getItem(`${p}/`)
      if (handle?.isDirectory()) {
        ;(handle as FileTreeDirectoryHandle).expand()
      }
    }
    const target = model.getItem(reveal) ? reveal : `${reveal}/`
    model.scrollToPath(target, { offset: 'center' })
    onRevealed()
  }, [reveal, kinds, model, onRevealed])

  return (
    <div className="shui-tree-wrap">
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
            data-autofocus=""
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
        {onCreate ? (
          <div className="shui-tree-new-actions">
            <button
              type="button"
              className="shui-tree-new-btn"
              aria-label="New file"
              title="new file"
              onClick={() => {
                setCreateKind((kind) => (kind === 'file' ? null : 'file'))
                setCreateError(null)
              }}
            >
              <FilePlus aria-hidden />
            </button>
            <button
              type="button"
              className="shui-tree-new-btn"
              aria-label="New folder"
              title="new folder"
              onClick={() => {
                setCreateKind((kind) => (kind === 'folder' ? null : 'folder'))
                setCreateError(null)
              }}
            >
              <FolderPlus aria-hidden />
            </button>
          </div>
        ) : null}
      </div>
      {createKind !== null ? (
        <div className="shui-tree-new-row">
          <input
            type="text"
            value={createName}
            onChange={(event) => setCreateName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') void submitCreate()
              if (event.key === 'Escape') {
                setCreateKind(null)
                setCreateName('')
                setCreateError(null)
              }
            }}
            placeholder={
              createKind === 'file' ? 'new/file/path.txt' : 'new/folder/path'
            }
            aria-label={
              createKind === 'file' ? 'New file path' : 'New folder path'
            }
            autoComplete="off"
            spellCheck={false}
            // biome-ignore lint/a11y/noAutofocus: the row exists because the user just asked to type a name
            autoFocus
          />
          {createError !== null ? (
            <div className="shui-tree-new-error">{createError}</div>
          ) : null}
        </div>
      ) : null}

      <div className="shui-tree-stage">
        {!tree ? (
          <div className="shui-side-note">loading tree…</div>
        ) : tree.paths.length === 0 ? (
          <div className="shui-side-note">
            {hiddenFiltered
              ? 'nothing visible — hidden entries are filtered'
              : 'empty folder'}
          </div>
        ) : filterMatchCount === 0 ? (
          <div className="shui-side-note">no matching files</div>
        ) : (
          <FileTree
            model={model}
            className="shui-tree"
            style={{ ...TREE_THEME, colorScheme: theme }}
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

      {tree && tree.truncations.length > 0 ? (
        <div className="shui-side-note ghost">
          partial listing — {tree.truncations.length}{' '}
          {tree.truncations.length === 1 ? 'folder' : 'folders'} truncated by
          depth/size limits
        </div>
      ) : null}
    </div>
  )
}

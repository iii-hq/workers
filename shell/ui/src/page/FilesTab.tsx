/* The files tab — @pierre/trees' FileTree over the worker's
   `coder::tree` listing. The tree renders into shadow DOM, so console
   classes can't reach it; theming rides the `--trees-*-override` CSS
   variables (custom properties inherit through shadow boundaries),
   mapped onto the console's design tokens.

   Open semantics: single click previews (selection change), double
   click pins — the dblclick is a composed DOM event, so it bubbles out
   of the tree's shadow root to the wrapper. Folder expansion is
   reported (debounced) for per-tab persistence and replayed through
   `resetPaths({ initialExpandedPaths })`. */

import type { FileTreeDirectoryHandle, GitStatusEntry } from '@pierre/trees'
import { FileTree, useFileTree } from '@pierre/trees/react'
import { useEffect, useMemo, useRef } from 'react'
import type { FlatTree } from './coder'
import { TREE_THEME } from './tree-theme'

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
  /** Single click on a file — open as the preview tab. */
  onPreviewFile: (relPath: string) => void
  /** Double click on a file — open pinned. */
  onPinFile: (relPath: string) => void
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
  onPreviewFile,
  onPinFile,
}: FilesTabProps) {
  // The model is created once per component lifetime; data arriving
  // later flows through resetPaths/setGitStatus below. Selection opens
  // through a ref so the creation-time callback never goes stale.
  const openRef = useRef<(paths: readonly string[]) => void>(() => {})
  const lastFileRef = useRef<string | null>(null)
  const { model } = useFileTree({
    paths: tree?.paths ?? [],
    onSelectionChange: (selected) => openRef.current(selected),
  })

  const kinds = tree?.kinds
  useEffect(() => {
    openRef.current = (selected) => {
      const path = selected[0]
      if (!path || !kinds) return
      if (kinds.get(path) === 'file') {
        lastFileRef.current = path
        onPreviewFile(path)
      } else {
        // A dir selection must not leave a stale file behind — the
        // wrapper's dblclick pin would hit the wrong path.
        lastFileRef.current = null
      }
    }
  }, [kinds, onPreviewFile])

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
    if (pathsKey === '' || pathsKey === lastPathsKeyRef.current) return
    lastPathsKeyRef.current = pathsKey
    // The model accepts dir ids in either spelling depending on how the
    // row materialized — hand it both.
    const initialExpandedPaths = expandedRef.current.flatMap((p) => [
      p,
      `${p}/`,
    ])
    model.resetPaths(tree?.paths ?? [], { initialExpandedPaths })
  }, [model, pathsKey, tree])

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

  if (!tree) {
    return <div className="shui-side-note">loading tree…</div>
  }
  if (tree.paths.length === 0) {
    return (
      <div className="shui-side-note">
        {hiddenFiltered ? '· nothing visible — hidden entries are filtered' : '· empty folder'}
      </div>
    )
  }

  return (
    <div
      className="shui-tree-wrap"
      onDoubleClick={() => {
        if (lastFileRef.current) onPinFile(lastFileRef.current)
      }}
    >
      <FileTree
        model={model}
        className="shui-tree"
        style={{ ...TREE_THEME, colorScheme: theme }}
      />
      {tree.truncations.length > 0 ? (
        <div className="shui-side-note ghost">
          partial listing — {tree.truncations.length}{' '}
          {tree.truncations.length === 1 ? 'folder' : 'folders'} truncated by
          depth/size limits
        </div>
      ) : null}
    </div>
  )
}

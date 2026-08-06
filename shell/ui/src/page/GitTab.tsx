/* The git tab — working-tree changes for the browsed root as a
   collapsible FileTree (folders expand/collapse; the tree's git-status
   decorations color each name). Selecting a FILE opens its diff in the
   main pane (rendered by the console's shared FileDiff); folders only
   toggle. `git status -uall` (git.ts) lists untracked files
   individually, so a brand-new folder shows its contents instead of one
   unclickable `folder/` record. */

import { FileTree, useFileTree } from '@pierre/trees/react'
import { useEffect, useMemo, useRef } from 'react'
import type { GitChange, GitState } from './git'
import { TREE_THEME } from './tree-theme'

interface GitTabProps {
  state: GitState | null
  theme: 'light' | 'dark'
  onSelect: (change: GitChange) => void
  onRefresh: () => void
}

export function GitTab({ state, theme, onSelect, onRefresh }: GitTabProps) {
  if (!state) {
    return <div className="shui-side-note">checking repository…</div>
  }
  if (state.kind === 'not-a-repo') {
    return <div className="shui-side-note">· not a git repository</div>
  }
  if (state.kind === 'error') {
    return (
      <div className="shui-side-note warn">
        git failed: {state.message}
        <button type="button" className="shui-linkish" onClick={onRefresh}>
          retry
        </button>
      </div>
    )
  }

  if (state.changes.length === 0) {
    return (
      <div className="shui-side-note">
        · working tree clean
        <button type="button" className="shui-linkish" onClick={onRefresh}>
          refresh
        </button>
      </div>
    )
  }

  return (
    <div className="shui-git-tree">
      <div className="shui-side-note head">
        {state.changes.length}{' '}
        {state.changes.length === 1 ? 'change' : 'changes'}
        <button type="button" className="shui-linkish" onClick={onRefresh}>
          refresh
        </button>
      </div>
      <GitChangesTree
        changes={state.changes}
        theme={theme}
        onSelect={onSelect}
      />
    </div>
  )
}

/** Every parent dir of every change, in both id spellings — the changes
    tree always starts fully expanded (it exists to show what changed). */
function allParentDirs(paths: readonly string[]): string[] {
  const dirs = new Set<string>()
  for (const path of paths) {
    let idx = path.indexOf('/')
    while (idx !== -1) {
      const dir = path.slice(0, idx)
      dirs.add(dir)
      dirs.add(`${dir}/`)
      idx = path.indexOf('/', idx + 1)
    }
  }
  return [...dirs]
}

function GitChangesTree({
  changes,
  theme,
  onSelect,
}: {
  changes: readonly GitChange[]
  theme: 'light' | 'dark'
  onSelect: (change: GitChange) => void
}) {
  // Defensive: a record that still names a directory (e.g. a submodule)
  // must not become a fake file row.
  const fileChanges = useMemo(
    () => changes.filter((c) => c.path !== '' && !c.path.endsWith('/')),
    [changes],
  )
  const byPath = useMemo(
    () => new Map(fileChanges.map((c) => [c.path, c] as const)),
    [fileChanges],
  )
  const byPathRef = useRef(byPath)
  byPathRef.current = byPath
  const onSelectRef = useRef(onSelect)
  onSelectRef.current = onSelect

  const { model } = useFileTree({
    paths: fileChanges.map((c) => c.path),
    onSelectionChange: (selected) => {
      const path = selected[0]
      if (!path) return
      const change = byPathRef.current.get(path)
      if (change) onSelectRef.current(change)
    },
  })

  const pathsKey = useMemo(
    () => fileChanges.map((c) => c.path).join('\n'),
    [fileChanges],
  )
  const lastPathsKeyRef = useRef('')
  useEffect(() => {
    if (pathsKey === lastPathsKeyRef.current) return
    lastPathsKeyRef.current = pathsKey
    const paths = fileChanges.map((c) => c.path)
    model.resetPaths(paths, { initialExpandedPaths: allParentDirs(paths) })
  }, [model, pathsKey, fileChanges])

  useEffect(() => {
    model.setGitStatus(
      fileChanges.map((c) => ({ path: c.path, status: c.status })),
    )
  }, [model, fileChanges])

  return (
    <FileTree
      model={model}
      className="shui-tree"
      style={{ ...TREE_THEME, colorScheme: theme }}
    />
  )
}

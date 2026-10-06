/* WebStorm's "Show Diff Preview": the file picked in the Log's commit, its
   diff under the commit's files, drawn by the editor's own diff tab. It
   reads only while it shows and once the pick rests; a read that lands
   after another file was picked is dropped. Its header leaves the path out: the file is the one
   selected right above it. */

import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useState } from 'react'
import { type DiffOptions, DiffTab, type DiffTabActions, type DiffTabState } from './DiffTab'
import { loadDiffContents } from './diff-load'
import type { CommitFile } from './git-log-window'
import { basename } from './paths'

// A commit's two sides are revisions: no turn is ever read.
const NO_TURNS = { get: async () => null }
const NO_ACTIONS: DiffTabActions = {}
const LOADING: DiffTabState = { phase: 'loading' }
// Each read is two `git show`s; the commit's details wait as long.
const PREVIEW_DEBOUNCE_MS = 150

/** The commit's change to `path` (below `root`), read as the editor's
    commit diff tabs read it: `parent` (null for a root commit) against
    `sha`, `from` naming a rename's source. */
export function useCommitFileDiff(
  host: Host,
  root: string,
  path: string,
  sha: string,
  parent: string | null,
  from: string | undefined,
): { state: DiffTabState; reload(): void } {
  const key = [root, path, sha, parent, from].join('\n')
  const [read, setRead] = useState<{ key: string; state: DiffTabState } | null>(null)
  const [attempt, setAttempt] = useState(0)
  // The key stands for what the read uses; attempt is the reload signal.
  // The file list selects on focus, so arrowing through it moves the key on
  // every row: read once the selection rests, as the commit's details do.
  useEffect(() => {
    let live = true
    const timer = setTimeout(() => {
      loadDiffContents(host, root, path, { type: 'commit', sha, parent, from }, NO_TURNS)
        .then<DiffTabState>((contents) => ({ phase: 'ready', contents }))
        .catch<DiffTabState>((error: unknown) => ({ phase: 'error', message: errorMessage(error) }))
        .then((state) => {
          if (live) setRead({ key, state })
        })
    }, PREVIEW_DEBOUNCE_MS)
    return () => {
      live = false
      clearTimeout(timer)
    }
  }, [host, key, attempt])
  const reload = useCallback(() => {
    setRead(null)
    setAttempt((value) => value + 1)
  }, [])
  return { state: read?.key === key ? read.state : LOADING, reload }
}

export function GitDiffPreview({
  host,
  root,
  file,
  sha,
  parent,
  options,
  onOptions,
}: {
  host: Host
  root: string
  file: CommitFile
  sha: string
  /** The commit's first parent; null for a root commit. */
  parent: string | null
  options: DiffOptions
  onOptions(next: DiffOptions): void
}) {
  const { state, reload } = useCommitFileDiff(host, root, file.view, sha, parent, file.from)
  return (
    <section className="shui-git-preview" aria-label={`Diff of ${basename(file.path)}`}>
      <DiffTab
        path={file.view}
        source={{ type: 'commit', sha, parent, from: file.from }}
        state={state}
        options={options}
        onOptionsChange={onOptions}
        onReload={reload}
        actions={NO_ACTIONS}
      />
    </section>
  )
}

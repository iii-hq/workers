/* The diff surface for git-tab selections — the console's shared
   FileDiff (the same diff renderer chat cards use), fed with the two
   full file bodies: HEAD content via `git show`, working-tree content
   via `coder::read-file`. */

import { FileDiff, type Host } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import { coderReadFile, joinPath } from './coder'
import { type GitChange, gitShowHead } from './git'

type DiffState =
  | { phase: 'loading' }
  | { phase: 'error'; message: string }
  | { phase: 'ready'; oldContents: string; newContents: string }

interface DiffPaneProps {
  host: Host
  root: string
  change: GitChange
  /** When set, diff against this text instead of git HEAD — the live
      view's baseline for files outside a repo. */
  baseline?: string
  /** When set, resolve the git baseline from THIS directory instead of
      the browsed root — the file's own repo when the browsed root sits
      above it (a worktree under the home directory). */
  gitDir?: string
}

export function DiffPane({ host, root, change, baseline, gitDir }: DiffPaneProps) {
  const [state, setState] = useState<DiffState>({ phase: 'loading' })
  const seqRef = useRef(0)

  useEffect(() => {
    const seq = ++seqRef.current
    setState({ phase: 'loading' })

    const oldPath = change.from ?? change.path
    const oldSide =
      baseline !== undefined
        ? Promise.resolve(baseline)
        : change.status === 'added' || change.status === 'untracked'
          ? Promise.resolve('')
          : gitDir !== undefined
            ? gitShowHead(host, gitDir, oldPath.slice(oldPath.lastIndexOf('/') + 1))
            : gitShowHead(host, root, oldPath)
    const newSide =
      change.status === 'deleted'
        ? Promise.resolve('')
        : coderReadFile(host, joinPath(root, change.path)).then((out) =>
            out.is_utf8 === false ? null : (out.content ?? ''),
          )

    Promise.all([oldSide, newSide])
      .then(([oldContents, newContents]) => {
        if (seqRef.current !== seq) return
        if (newContents === null) {
          setState({ phase: 'error', message: 'binary file — no text diff' })
          return
        }
        setState({ phase: 'ready', oldContents, newContents })
      })
      .catch((err: unknown) => {
        if (seqRef.current !== seq) return
        setState({
          phase: 'error',
          message: errorMessage(err),
        })
      })
  }, [host, root, change, baseline, gitDir])

  return (
    <div className="shui-main-pane">
      <div className="shui-editor-head">
        <span className="path" title={change.path}>
          {change.from ? `${change.from} → ${change.path}` : change.path}
        </span>
        <span className="meta">{change.status}</span>
      </div>
      <div className="shui-editor-body">
        {state.phase === 'loading' ? (
          <div className="shui-side-note">loading diff…</div>
        ) : state.phase === 'error' ? (
          <div className="shui-side-note warn">{state.message}</div>
        ) : (
          <FileDiff
            oldFile={{
              name: change.from ?? change.path,
              contents: state.oldContents,
            }}
            newFile={{ name: change.path, contents: state.newContents }}
          />
        )}
      </div>
    </div>
  )
}

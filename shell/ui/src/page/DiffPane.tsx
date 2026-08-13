/* The diff surface for git-tab selections and live follows — shell's
   OWN renderer (no shared component): Myers line diff with folded
   unmodified context, dual line-number gutters, syntax-colored lines,
   and intraline change emphasis. Baselines: git HEAD via `git show`
   (from the browsed root or the file's own repo), or a text baseline
   the page supplies; the new side is the working tree via
   `coder::read-file`. */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import { langFromPath, renderCodeLine } from '../lib/syntax'
import { coderReadFile, joinPath } from './coder'
import { type DiffOp, type DiffRow, diffLines, diffTotals, foldRows } from './diff'
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
        const raw = errorMessage(err)
        setState({
          phase: 'error',
          // A followed file can vanish between the event and the read
          // (renamed away, deleted) — say that instead of a raw C211.
          message: raw.includes('not found or not accessible')
            ? 'file no longer exists on disk'
            : raw,
        })
      })
  }, [host, root, change, baseline, gitDir])

  const ops = useMemo(
    () =>
      state.phase === 'ready' ? diffLines(state.oldContents, state.newContents) : null,
    [state],
  )
  const totals = useMemo(() => (ops !== null ? diffTotals(ops) : null), [ops])
  const rows = useMemo(() => (ops !== null ? foldRows(ops) : null), [ops])

  const verb =
    change.status === 'added' || change.status === 'untracked'
      ? 'Added'
      : change.status === 'deleted'
        ? 'Deleted'
        : change.status === 'renamed'
          ? 'Renamed'
          : 'Edited'
  return (
    <div className="shui-main-pane">
      <div className="shui-editor-head">
        <span className="shui-diff-verb">{verb}</span>
        <span className="path" title={change.path}>
          {change.from ? `${change.from} → ${change.path}` : change.path}
        </span>
        {totals !== null ? (
          <span className="shui-diff-stats">
            (<span className="add">+{totals.add}</span> <span className="del">−{totals.del}</span>)
          </span>
        ) : null}
      </div>
      <div className="shui-editor-body">
        {state.phase === 'loading' ? (
          <div className="shui-side-note">loading diff…</div>
        ) : state.phase === 'error' ? (
          <div className="shui-side-note warn">{state.message}</div>
        ) : rows !== null && totals !== null && totals.add === 0 && totals.del === 0 ? (
          <div className="shui-side-note">· no line changes</div>
        ) : rows !== null ? (
          <DiffBody rows={rows} lang={langFromPath(change.path)} />
        ) : null}
      </div>
    </div>
  )
}

/** Past this many visible rows the per-line tokenizer is skipped — the
    same guard the reference TUI applies to keep huge diffs instant. */
const HIGHLIGHT_ROW_LIMIT = 3000

function DiffBody({ rows, lang }: { rows: DiffRow[]; lang: string }) {
  // Folds expand in place and stay expanded for this diff instance.
  const [openFolds, setOpenFolds] = useState<ReadonlySet<number>>(new Set())
  const effLang = rows.length > HIGHLIGHT_ROW_LIMIT ? 'plain' : lang
  return (
    <div className="shui-diff">
      {rows.map((row) => {
        if (row.kind === 'fold') {
          if (openFolds.has(row.index)) {
            return row.ops.map((op) => <DiffLine key={opKey(op)} op={op} lang={effLang} />)
          }
          return (
            <button
              key={`fold:${row.index}`}
              type="button"
              className="shui-diff-fold"
              onClick={() => setOpenFolds((prev) => new Set(prev).add(row.index))}
            >
              ⌄ {row.count} unmodified {row.count === 1 ? 'line' : 'lines'}
            </button>
          )
        }
        return <DiffLine key={opKey(row.op)} op={row.op} lang={effLang} />
      })}
    </div>
  )
}

function opKey(op: DiffOp): string {
  return `${op.type}:${'oldLine' in op ? op.oldLine : 0}:${'newLine' in op ? op.newLine : 0}`
}

/** One gutter, the way the reference renders it: the NEW line number for
    additions and context, the OLD one for deletions — the sign column
    disambiguates. */
function DiffLine({ op, lang }: { op: DiffOp; lang: string }) {
  const marker = op.type === 'add' ? '+' : op.type === 'del' ? '−' : ' '
  const ln = op.type === 'del' ? op.oldLine : op.newLine
  return (
    <div className={`shui-diff-line ${op.type}`}>
      <span className="ln">{ln}</span>
      <span className="mark">{marker}</span>
      <span className="code">
        {renderCodeLine(op.text, lang, op.type === 'same' ? undefined : op.hl)}
      </span>
    </div>
  )
}

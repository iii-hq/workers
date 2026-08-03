/**
 * A collapsible view of a JSON value.
 *
 * `JsonHighlight` takes `{code: string}` — flat text that cannot collapse a
 * branch, summarise an array as `[…12 items]`, or copy a subtree. For a column
 * holding a large document that is the difference between reading the shape
 * and scrolling past it.
 *
 * Leaves reuse the grid's `.db-cell-*` classes on purpose, so a value looks the
 * same here as it does in the table it came from.
 */

import { useState } from 'react'
import { cellText } from '../lib/grid-cursor'
import { ChevronRight, Copy } from './icons'

/** Children rendered before the tail summary. Long arrays are common. */
const MAX_CHILDREN = 100

type Json = unknown

export function JsonTree({ value, onCopy }: { value: Json; onCopy?: (text: string) => void }) {
  // Expansion is held by path key so it survives a re-render, and the root
  // starts open — collapsing the thing you just opened is not useful.
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set(['$']))

  const toggle = (path: string) =>
    setOpen((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })

  return (
    <div className="db-json">
      <Node path="$" label={null} value={value} depth={0} open={open} onToggle={toggle} onCopy={onCopy} />
    </div>
  )
}

function Node({
  path,
  label,
  value,
  depth,
  open,
  onToggle,
  onCopy,
}: {
  path: string
  label: string | null
  value: Json
  depth: number
  open: ReadonlySet<string>
  onToggle: (path: string) => void
  onCopy?: (text: string) => void
}) {
  const branch = isBranch(value)
  const expanded = open.has(path)
  const entries = branch ? entriesOf(value) : []
  const shown = entries.slice(0, MAX_CHILDREN)
  const hidden = entries.length - shown.length

  return (
    <div className="db-json-node">
      <div className="db-json-row" style={{ paddingLeft: `${depth * 12}px` }}>
        {branch ? (
          <button
            type="button"
            className={`db-json-twist${expanded ? ' open' : ''}`}
            onClick={() => onToggle(path)}
            aria-expanded={expanded}
            aria-label={expanded ? `collapse ${label ?? 'root'}` : `expand ${label ?? 'root'}`}
          >
            <ChevronRight size={10} aria-hidden />
          </button>
        ) : (
          <span className="db-json-twist leaf" />
        )}

        {label !== null ? <span className="db-json-key">{label}</span> : null}

        {branch ? (
          <span className="db-json-summary">{summary(value, entries.length, expanded)}</span>
        ) : (
          <Leaf value={value} />
        )}

        {onCopy ? (
          <button
            type="button"
            className="db-json-copy"
            title="copy this value"
            onClick={() => onCopy(branch ? safeStringify(value) : cellText(value))}
          >
            <Copy size={10} aria-hidden />
          </button>
        ) : null}
      </div>

      {branch && expanded
        ? shown.map(([k, v]) => (
            <Node
              key={k}
              path={`${path}.${k}`}
              label={k}
              value={v}
              depth={depth + 1}
              open={open}
              onToggle={onToggle}
              onCopy={onCopy}
            />
          ))
        : null}

      {branch && expanded && hidden > 0 ? (
        <div className="db-json-more" style={{ paddingLeft: `${(depth + 1) * 12 + 14}px` }}>
          {hidden} more not shown
        </div>
      ) : null}
    </div>
  )
}

/** Leaves match the grid, so a value reads identically in both places. */
function Leaf({ value }: { value: Json }) {
  if (value === null) return <span className="db-cell-null">null</span>
  if (value === undefined) return <span className="db-cell-null">undefined</span>
  if (typeof value === 'boolean') {
    return <span className={value ? 'db-cell-bool-true' : 'db-cell-bool-false'}>{String(value)}</span>
  }
  if (typeof value === 'number') return <span className="db-cell-num">{String(value)}</span>
  // An empty string must not render as nothing at all.
  if (value === '') return <span className="db-cell-null">''</span>
  return <span className="db-cell-str">{String(value)}</span>
}

function isBranch(v: Json): v is Record<string, unknown> | unknown[] {
  return typeof v === 'object' && v !== null
}

function entriesOf(v: Record<string, unknown> | unknown[]): [string, unknown][] {
  return Array.isArray(v) ? v.map((item, i) => [String(i), item]) : Object.entries(v)
}

function summary(v: Record<string, unknown> | unknown[], count: number, expanded: boolean): string {
  const array = Array.isArray(v)
  if (expanded) return array ? '[' : '{'
  const noun = array ? (count === 1 ? 'item' : 'items') : count === 1 ? 'key' : 'keys'
  return array ? `[… ${count} ${noun}]` : `{… ${count} ${noun}}`
}

function safeStringify(v: Json): string {
  try {
    return JSON.stringify(v, null, 2)
  } catch {
    return String(v)
  }
}

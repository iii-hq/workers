/**
 * The focused cell, in full.
 *
 * A pure function of the grid cursor: arrows keep driving the grid, this
 * follows. That is why it can sit beside the grid without competing for the
 * keyboard.
 *
 * Two measurements are worth the lines. **Code points, not `.length`** —
 * `'👋'.length` is 2, which is the kind of number that makes someone doubt the
 * whole panel; `[...s].length` is 1. And **bytes via `TextEncoder`**, because
 * a `varchar(20)` limit counts bytes on some drivers and characters on others,
 * and "why did this fail to insert" usually ends here.
 */

import { Badge, Button, MetaRow } from '@iii-dev/console-ui'
import { copyText } from '@iii-dev/console-ui/format'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, Copy, Link2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { cellText } from '../lib/grid-cursor'
import type { ForeignKeyRef } from '../lib/rpc'
import { JsonTree } from './JsonTree'

const ENCODER = new TextEncoder()

export function CellInspector({
  table,
  column,
  type,
  value,
  row,
  rowCount,
  col,
  colCount,
  foreignKey,
  onFollow,
}: {
  table: string
  column: string
  type?: string
  value: unknown
  row: number
  rowCount: number
  col: number
  colCount: number
  foreignKey?: ForeignKeyRef
  onFollow?: (fk: ForeignKeyRef, value: unknown) => void
}) {
  const [wrap, setWrap] = useState(true)

  const text = useMemo(() => cellText(value), [value])
  const isNull = value === null || value === undefined
  const json = useMemo(() => asJson(value), [value])
  // Recolour briefly rather than raising a toast — the house pattern.
  const valueCopy = useCopyFlash(text, 1200)

  const codePoints = isNull ? 0 : [...text].length
  const bytes = isNull ? 0 : ENCODER.encode(text).length

  return (
    <div className="db-inspect">
      <div className="db-inspect-head">
        <span className="db-inspect-path">
          {table}.{column}
        </span>
        {type ? <Badge variant="default">{type}</Badge> : null}
      </div>

      <div className="db-inspect-pos">
        Row {row + 1} of {rowCount} · Column {col + 1} of {colCount}
      </div>

      <MetaRow
        items={[
          { label: 'Kind', value: kindOf(value) },
          { label: 'Characters', value: codePoints.toLocaleString() },
          { label: 'Bytes', value: bytes.toLocaleString() },
        ]}
      />

      {foreignKey && onFollow && !isNull ? (
        <Button
          variant="ghost"
          size="sm"
          onClick={() => onFollow(foreignKey, value)}
        >
          <Link2 size={16} aria-hidden />
          Show {foreignKey.table} where {foreignKey.column} = {text}
        </Button>
      ) : null}

      <div className="db-inspect-value">
        {isNull ? (
          <span className="db-cell-null">NULL</span>
        ) : value === '' ? (
          // Distinct from NULL, and from an empty box.
          <span className="db-cell-null">'' (empty string)</span>
        ) : json !== undefined ? (
          <JsonTree value={json} onCopy={(t) => void copyText(t)} />
        ) : (
          <pre className={`db-inspect-pre${wrap ? ' wrap' : ''}`}>{text}</pre>
        )}
      </div>

      <div className="db-inspect-actions">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => valueCopy.copy()}
          disabled={isNull}
        >
          {valueCopy.state === 'copied' ? (
            <Check size={16} aria-hidden />
          ) : (
            <Copy size={16} aria-hidden />
          )}
          {valueCopy.state === 'copied' ? 'Copied' : 'Copy value'}
        </Button>
        {json === undefined && text.length > 80 ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setWrap((w) => !w)}
          >
            {wrap ? 'No wrap' : 'Wrap'}
          </Button>
        ) : null}
      </div>
    </div>
  )
}

function kindOf(value: unknown): string {
  if (value === null || value === undefined) return 'null'
  if (Array.isArray(value)) return 'array'
  return typeof value
}

/**
 * The value as JSON, when it is worth a tree.
 *
 * Objects qualify directly. A *string* qualifies only when it parses to an
 * object or array — many drivers hand back JSON columns as text, and rendering
 * that as a flat line loses the structure the column exists to hold. A string
 * that parses to a bare number is left alone: `"5"` is not a document.
 */
function asJson(value: unknown): unknown {
  if (value !== null && typeof value === 'object') return value
  if (typeof value !== 'string') return undefined
  const trimmed = value.trim()
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return undefined
  try {
    const parsed = JSON.parse(trimmed)
    return typeof parsed === 'object' && parsed !== null ? parsed : undefined
  } catch {
    return undefined
  }
}

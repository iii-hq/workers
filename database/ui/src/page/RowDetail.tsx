/**
 * Inspector for one selected row: every column with its full (untruncated)
 * value, JSON pretty-printed, one copy button per field. Complements the
 * grid, which truncates long values to keep rows scannable. Rendered inside
 * the inspector's tabbed slot, so the container and the close button belong
 * to the parent.
 */

import { IconButton, JsonHighlight } from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, Copy, KeyRound } from 'lucide-react'
import { cellText } from '../lib/grid-cursor'
import type { ColumnInfo } from './db-data'

interface RowDetailProps {
  row: Record<string, unknown>
  columns: ColumnInfo[]
}

export function RowDetail({ row, columns }: RowDetailProps) {
  const names =
    columns.length > 0 ? columns.map((c) => c.name) : Object.keys(row)
  const byName = new Map(columns.map((c) => [c.name, c]))

  return (
    <div className="db-rowdetail-body">
      {names.map((name) => {
        const col = byName.get(name)
        const value = row[name]
        const isJson =
          value !== null && value !== undefined && typeof value === 'object'
        return (
          <div key={name} className="db-field">
            <div className="db-field-head">
              {col?.pk ? (
                <KeyRound size={16} style={{ color: 'var(--color-accent)' }} />
              ) : null}
              <span className="db-field-name">{name}</span>
              {col?.type ? (
                <span className="db-field-type">{col.type}</span>
              ) : null}
              <FieldCopy name={name} value={value} />
            </div>
            <div className="db-field-value">
              {value === null || value === undefined ? (
                <span className="db-cell-null">NULL</span>
              ) : isJson ? (
                <JsonHighlight code={JSON.stringify(value, null, 2)} />
              ) : typeof value === 'boolean' ? (
                <span
                  className={value ? 'db-cell-bool-true' : 'db-cell-bool-false'}
                >
                  {String(value)}
                </span>
              ) : (
                <span className="db-cell-str">{String(value)}</span>
              )}
            </div>
            {col?.fkTarget ? (
              <div className="db-field-fk">References {col.fkTarget}</div>
            ) : null}
          </div>
        )
      })}
    </div>
  )
}

/** One field's copy affordance; the flash is per field, so each owns a hook. */
function FieldCopy({ name, value }: { name: string; value: unknown }) {
  const { state, copy } = useCopyFlash(
    value == null ? 'NULL' : cellText(value),
    1200,
  )
  return (
    <IconButton label={`copy ${name}`} onClick={() => copy()}>
      {state === 'copied' ? <Check size={16} /> : <Copy size={16} />}
    </IconButton>
  )
}

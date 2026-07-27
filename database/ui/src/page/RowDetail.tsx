/**
 * Inspector for one selected row: every column with its full (untruncated)
 * value, JSON pretty-printed, one copy button per field. Complements the
 * grid, which truncates long values to keep rows scannable.
 */

import { Button, JsonHighlight } from '@iii-dev/console-ui'
import { useState } from 'react'
import { cellText, copyText } from './cells'
import type { ColumnInfo } from './db-data'
import { Check, Copy, KeyRound, X } from './icons'

interface RowDetailProps {
  table: string
  row: Record<string, unknown>
  columns: ColumnInfo[]
  onClose: () => void
}

export function RowDetail({ table, row, columns, onClose }: RowDetailProps) {
  const [copied, setCopied] = useState<string | null>(null)
  const names =
    columns.length > 0 ? columns.map((c) => c.name) : Object.keys(row)
  const byName = new Map(columns.map((c) => [c.name, c]))

  const copyValue = (name: string) => {
    copyText(cellText(row[name]))
    setCopied(name)
    window.setTimeout(() => {
      setCopied((cur) => (cur === name ? null : cur))
    }, 1200)
  }

  return (
    <aside className="db-rowdetail">
      <div className="db-rowdetail-head">
        <span className="cap">row · {table}</span>
        <Button
          variant="icon"
          size="icon"
          onClick={onClose}
          aria-label="close row detail"
        >
          <X size={14} />
        </Button>
      </div>
      {names.map((name) => {
        const col = byName.get(name)
        const value = row[name]
        const isJson =
          value !== null && value !== undefined && typeof value === 'object'
        return (
          <div key={name} className="db-field">
            <div className="db-field-head">
              {col?.pk ? (
                <KeyRound size={10} style={{ color: 'var(--color-accent)' }} />
              ) : null}
              <span className="db-field-name">{name}</span>
              {col?.type ? (
                <span className="db-field-type">{col.type}</span>
              ) : null}
              <button
                type="button"
                className={`db-field-copy${copied === name ? ' copied' : ''}`}
                onClick={() => copyValue(name)}
                aria-label={`copy ${name}`}
              >
                {copied === name ? <Check size={12} /> : <Copy size={12} />}
              </button>
            </div>
            <div className="db-field-value">
              {value === null || value === undefined ? (
                <span className="null">NULL</span>
              ) : isJson ? (
                <JsonHighlight code={JSON.stringify(value, null, 2)} />
              ) : typeof value === 'boolean' ? (
                <span className={value ? 'bool-true' : 'bool-false'}>
                  {String(value)}
                </span>
              ) : (
                <span className="plain">{String(value)}</span>
              )}
            </div>
            {col?.fkTarget ? (
              <div className="db-field-fk">references {col.fkTarget}</div>
            ) : null}
          </div>
        )
      })}
    </aside>
  )
}

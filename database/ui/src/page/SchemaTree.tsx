/**
 * Sidebar schema explorer, grouped into tables and views. Each entry expands
 * to its column list (name, type, PK/FK markers) plus indexes for tables,
 * fetched lazily per table (through the tab's `host`) and cached for the life
 * of the component — the parent remounts it on db switch / refresh, which is
 * the cache-invalidation point.
 */

import { type Host, Input, Skeleton } from '@iii-dev/console-ui'
import { type ComponentType, useState } from 'react'
import {
  type ColumnInfo,
  commonSchema,
  type DbDriver,
  type DbTable,
  type IndexInfo,
  tableColumns,
  tableIndexes,
} from './db-data'
import {
  ChevronRight,
  Eye,
  type IconProps,
  KeyRound,
  Link2,
  Table2,
} from './icons'

interface SchemaTreeProps {
  host: Host
  db: string
  driver: DbDriver
  tables: DbTable[]
  selectedTable: string | null
  onSelectTable: (name: string) => void
}

interface TableSchema {
  columns: ColumnInfo[]
  indexes: IndexInfo[]
}

/** Arrow-key roving across a group's table/view name buttons — the tree has
    no built-in keyboard nav otherwise, only Tab-per-row. */
function roveTreeRow(e: React.KeyboardEvent<HTMLUListElement>) {
  if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
  const target = e.target
  if (!(target instanceof HTMLElement) || !target.matches('.db-tree-name'))
    return
  const rows = Array.from(
    e.currentTarget.querySelectorAll<HTMLButtonElement>('.db-tree-name'),
  )
  const i = rows.indexOf(target as HTMLButtonElement)
  if (i === -1) return
  e.preventDefault()
  rows[
    (i + (e.key === 'ArrowDown' ? 1 : -1) + rows.length) % rows.length
  ]?.focus()
}

export function SchemaTree({
  host,
  db,
  driver,
  tables,
  selectedTable,
  onSelectTable,
}: SchemaTreeProps) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [schemaByTable, setSchemaByTable] = useState<
    Record<string, TableSchema | 'loading' | 'error'>
  >({})

  // Fourteen `public.` prefixes ate the width the actual names needed.
  // Display strips a universal schema; selection, fetches and titles keep
  // the qualified name.
  const prefix = commonSchema(tables.map((t) => t.name))
  const display = (name: string) =>
    prefix && name.startsWith(`${prefix}.`)
      ? name.slice(prefix.length + 1)
      : name

  // Type-to-filter beats walking two tab stops per row. Matches the
  // displayed name and the qualified one; Escape clears.
  const [query, setQuery] = useState('')
  const needle = query.trim().toLowerCase()
  const matches = (t: DbTable) =>
    needle === '' ||
    display(t.name).toLowerCase().includes(needle) ||
    t.name.toLowerCase().includes(needle)
  const anyMatch = tables.some(matches)

  const toggle = (table: DbTable) => {
    setExpanded((cur) => {
      const next = new Set(cur)
      if (next.has(table.name)) {
        next.delete(table.name)
      } else {
        next.add(table.name)
      }
      return next
    })
    // Retry on re-expand after a failure; a cached schema stays cached.
    const cached = schemaByTable[table.name]
    if (cached === undefined || cached === 'error') {
      setSchemaByTable((cur) => ({ ...cur, [table.name]: 'loading' }))
      void Promise.all([
        tableColumns(host, db, driver, table.name),
        table.kind === 'table'
          ? tableIndexes(host, db, driver, table.name).catch(() => [])
          : Promise.resolve([]),
      ])
        .then(([columns, indexes]) => {
          setSchemaByTable((cur) => ({
            ...cur,
            [table.name]: { columns, indexes },
          }))
        })
        .catch(() => {
          setSchemaByTable((cur) => ({ ...cur, [table.name]: 'error' }))
        })
    }
  }

  const groups: {
    label: string
    kind: DbTable['kind']
    icon: ComponentType<IconProps>
  }[] = [
    { label: 'Tables', kind: 'table', icon: Table2 },
    { label: 'Views', kind: 'view', icon: Eye },
  ]

  return (
    <div className="db-tree">
      <div className="db-tree-filter">
        <Input
          value={query}
          onChange={setQuery}
          placeholder="Filter tables"
          aria-label="filter tables"
          data-autofocus=""
          data-db-tables-filter=""
          onKeyDown={(e) => {
            if (e.key === 'Escape') setQuery('')
          }}
        />
      </div>
      {!anyMatch && needle !== '' ? (
        <p className="db-tree-msg">No tables match "{needle}"</p>
      ) : null}
      {groups.map((group) => {
        const entries = tables.filter(
          (t) => t.kind === group.kind && matches(t),
        )
        if (entries.length === 0) return null
        return (
          <div key={group.kind}>
            {group.kind === 'view' ? (
              <div className="db-tree-grouphead">Views · {entries.length}</div>
            ) : null}
            <ul onKeyDown={roveTreeRow}>
              {entries.map((table) => {
                const isOpen = expanded.has(table.name)
                const isSelected = selectedTable === table.name
                const schema = schemaByTable[table.name]
                const Icon = group.icon
                return (
                  <li key={table.name}>
                    <div
                      className={`db-tree-row${isSelected ? ' active' : ''}`}
                    >
                      <button
                        type="button"
                        className="db-tree-toggle"
                        onClick={() => toggle(table)}
                        aria-expanded={isOpen}
                        aria-label={
                          isOpen
                            ? `collapse ${table.name} columns`
                            : `expand ${table.name} columns`
                        }
                      >
                        <ChevronRight
                          size={16}
                          style={{
                            transform: isOpen ? 'rotate(90deg)' : undefined,
                            transition: 'transform 0.12s',
                          }}
                        />
                      </button>
                      <button
                        type="button"
                        className="db-tree-name"
                        onClick={() => onSelectTable(table.name)}
                        title={table.name}
                      >
                        <Icon
                          size={16}
                          style={{ color: 'var(--color-ink-ghost)' }}
                        />
                        <span className="db-trunc">{display(table.name)}</span>
                      </button>
                    </div>
                    {isOpen ? <TableSchemaRows schema={schema} /> : null}
                  </li>
                )
              })}
            </ul>
          </div>
        )
      })}
    </div>
  )
}

function TableSchemaRows({
  schema,
}: {
  schema: TableSchema | 'loading' | 'error' | undefined
}) {
  if (schema === 'loading' || schema === undefined) {
    return (
      <div style={{ padding: '4px 12px 4px 36px' }}>
        <Skeleton style={{ display: 'block', height: 14, width: 96 }} />
      </div>
    )
  }
  if (schema === 'error') {
    return <p className="db-tree-msg alert">Failed to read schema</p>
  }
  return (
    <ul className="db-cols">
      {schema.columns.length === 0 ? (
        <li className="db-tree-msg">No columns</li>
      ) : (
        schema.columns.map((col) => (
          <li
            key={col.name}
            className="db-col"
            title={[
              col.type,
              col.nullable ? 'nullable' : 'not null',
              col.pk ? 'primary key' : null,
              col.fkTarget ? `references ${col.fkTarget}` : null,
            ]
              .filter(Boolean)
              .join(' · ')}
          >
            {col.pk ? (
              <KeyRound size={16} style={{ color: 'var(--color-accent)' }} />
            ) : col.fkTarget ? (
              <Link2 size={16} style={{ color: 'var(--color-ink-ghost)' }} />
            ) : (
              <span style={{ width: 10, flexShrink: 0 }} />
            )}
            <span className="db-col-name">{col.name}</span>
            <span className="db-col-type">{col.type}</span>
          </li>
        ))
      )}
      {schema.indexes.length > 0 ? (
        <>
          <li className="db-idx-head">Indexes</li>
          {schema.indexes.map((idx) => (
            <li
              key={idx.name}
              className="db-idx"
              title={[idx.unique ? 'unique' : null, idx.detail]
                .filter(Boolean)
                .join(' · ')}
            >
              <span style={{ width: 10, flexShrink: 0 }} />
              <span className="db-idx-name">{idx.name}</span>
              {idx.unique ? (
                <span className="db-idx-unique">Unique</span>
              ) : null}
            </li>
          ))}
        </>
      ) : null}
    </ul>
  )
}

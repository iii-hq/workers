/**
 * Sidebar schema explorer, grouped into tables and views. Each entry expands
 * to its column list (name, type, PK/FK markers) plus indexes for tables,
 * fetched lazily per table (through the tab's `host`) and cached for the life
 * of the component — the parent remounts it on db switch / refresh, which is
 * the cache-invalidation point.
 */

import { type Host, Skeleton } from '@iii-dev/console-ui'
import { type ComponentType, useState } from 'react'
import {
  type ColumnInfo,
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
    if (schemaByTable[table.name] === undefined) {
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
    { label: 'tables', kind: 'table', icon: Table2 },
    { label: 'views', kind: 'view', icon: Eye },
  ]

  return (
    <div className="db-tree">
      {groups.map((group) => {
        const entries = tables.filter((t) => t.kind === group.kind)
        if (entries.length === 0) return null
        return (
          <div key={group.kind}>
            {group.kind === 'view' ? (
              <div className="db-tree-grouphead">views · {entries.length}</div>
            ) : null}
            <ul>
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
                        aria-label={
                          isOpen
                            ? `collapse ${table.name} columns`
                            : `expand ${table.name} columns`
                        }
                      >
                        <ChevronRight
                          size={12}
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
                          size={12}
                          style={{ color: 'var(--color-ink-ghost)' }}
                        />
                        <span className="db-trunc">{table.name}</span>
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
    return <p className="db-tree-msg alert">failed to read schema</p>
  }
  return (
    <ul className="db-cols">
      {schema.columns.length === 0 ? (
        <li className="db-tree-msg">no columns</li>
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
              <KeyRound size={10} style={{ color: 'var(--color-accent)' }} />
            ) : col.fkTarget ? (
              <Link2 size={10} style={{ color: 'var(--color-ink-ghost)' }} />
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
          <li className="db-idx-head">indexes</li>
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
                <span className="db-idx-unique">unique</span>
              ) : null}
            </li>
          ))}
        </>
      ) : null}
    </ul>
  )
}

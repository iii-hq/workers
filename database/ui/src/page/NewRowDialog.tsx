/**
 * New row: one field per column, typed by the column — a number field for
 * numbers, a date picker for dates, true/false for booleans, a JSON box for
 * JSON — each starting on "Default" when the database can fill it in.
 * A foreign-key column gets "Select row", which swaps the form for a
 * filterable view of the referenced table; clicking a row takes its key.
 *
 * The insert goes through `database::insertRow`: the worker quotes the
 * identifiers and casts each value to its column's type, and returns the row
 * as stored. Only tables offer this; views never do.
 */

import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  type Host,
  Input,
  Select,
  StatusPanel,
} from '@iii-dev/console-ui'
import { ChevronLeft, ChevronRight, CircleAlert, KeyRound, Link2 } from 'lucide-react'
import { type ReactNode, useCallback, useMemo, useState } from 'react'
import { errText } from '../lib/errors'
import { cellText } from '../lib/grid-cursor'
import { type FilterSpec, isComplete, insertRow } from '../lib/rpc'
import type { SqlDialect } from '../lib/sql'
import { browseTableRef, type ColumnInfo, splitTableRef, tableColumns } from './db-data'
import { FilterBar } from './FilterBar'
import {
  buildInsertValues,
  type FieldMode,
  type FieldState,
  fkTable,
  hasServerDefault,
  type InsertColumn,
  initialField,
  inputKindFor,
  modesFor,
} from './new-row'
import { ResultGrid } from './result-grid'
import { useDatabaseRead } from './useDatabaseRead'

const PICKER_PAGE = 25

export function toInsertColumn(c: ColumnInfo): InsertColumn {
  return { name: c.name, type: c.type, nullable: c.nullable, pk: c.pk, defaultValue: c.defaultValue, fk: c.fk }
}

export function NewRowDialog({
  host,
  db,
  table,
  columns,
  dialect,
  open,
  onOpenChange,
  onInserted,
}: {
  host: Host
  db: string
  /** The page's identifier (schema-qualified on postgres). */
  table: string
  columns: readonly ColumnInfo[]
  dialect?: SqlDialect
  open: boolean
  onOpenChange: (open: boolean) => void
  onInserted: (row: Record<string, unknown> | null) => void
}) {
  const insertColumns = useMemo(() => columns.map(toInsertColumn), [columns])
  const [fields, setFields] = useState<Record<string, FieldState>>({})
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const [picking, setPicking] = useState<InsertColumn | null>(null)

  const field = (col: InsertColumn) => fields[col.name] ?? initialField(col)
  const setField = (col: InsertColumn, next: FieldState) => {
    setFields((cur) => ({ ...cur, [col.name]: next }))
    setErrors((cur) => {
      if (!(col.name in cur)) return cur
      const copy = { ...cur }
      delete copy[col.name]
      return copy
    })
  }

  const reset = () => {
    setFields({})
    setErrors({})
    setFailure(null)
    setPicking(null)
  }

  const submit = async () => {
    const built = buildInsertValues(insertColumns, fields)
    setErrors(built.errors)
    if (Object.keys(built.errors).length > 0) return
    setBusy(true)
    setFailure(null)
    try {
      const { schema, name } = splitTableRef(table)
      const res = await insertRow(host, db, name, schema, built.values)
      onInserted(res.row ?? null)
      reset()
      onOpenChange(false)
    } catch (err) {
      setFailure(errText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) reset()
        onOpenChange(next)
      }}
    >
      <DialogContent className="db-newrow">
        <DialogTitle>{picking ? `Select a row of ${fkTable(picking.fk!)}` : `New row in ${table}`}</DialogTitle>
        <DialogDescription>
          {picking
            ? `Click a row to use its ${picking.fk?.column} for ${picking.name}. Filters narrow the list.`
            : 'Fields left on Default take the database’s default. Values are cast to each column’s type by the worker.'}
        </DialogDescription>
        {picking?.fk ? (
          <RowPicker
            host={host}
            db={db}
            table={fkTable(picking.fk)}
            column={picking.fk.column}
            dialect={dialect}
            onBack={() => setPicking(null)}
            onPick={(value) => {
              setField(picking, { mode: 'value', text: cellText(value) })
              setPicking(null)
            }}
          />
        ) : (
          <form
            className="db-newrow-form"
            onSubmit={(event) => {
              event.preventDefault()
              void submit()
            }}
          >
            <div className="db-newrow-fields">
              {insertColumns.map((col) => (
                <FieldRow
                  key={col.name}
                  col={col}
                  state={field(col)}
                  error={errors[col.name]}
                  disabled={busy}
                  onChange={(next) => setField(col, next)}
                  onPick={col.fk ? () => setPicking(col) : undefined}
                />
              ))}
            </div>
            {failure ? (
              <StatusPanel
                variant="alert"
                icon={<CircleAlert size={18} />}
                headline="Insert failed"
                detail={failure}
              />
            ) : null}
            <div className="db-newrow-actions">
              {Object.keys(errors).length > 0 ? (
                <span className="db-newrow-note">
                  {Object.keys(errors).length} field{Object.keys(errors).length === 1 ? '' : 's'} to fix
                </span>
              ) : null}
              <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button type="submit" size="sm" disabled={busy}>
                {busy ? 'Inserting…' : 'Insert row'}
              </Button>
            </div>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}

const MODE_LABEL: Record<FieldMode, string> = {
  default: 'Default',
  null: 'NULL',
  value: 'Value',
}

function FieldRow({
  col,
  state,
  error,
  disabled,
  onChange,
  onPick,
}: {
  col: InsertColumn
  state: FieldState
  error?: string
  disabled: boolean
  onChange: (next: FieldState) => void
  onPick?: () => void
}) {
  const kind = inputKindFor(col.type)
  const modes = modesFor(col)
  const id = `db-newrow-${col.name}`
  const setText = (text: string) => onChange({ mode: 'value', text })
  const defaultLabel =
    col.defaultValue != null && col.defaultValue !== ''
      ? `Default · ${col.defaultValue}`
      : hasServerDefault(col)
        ? 'Default · generated'
        : 'Default · NULL'

  let control: ReactNode
  if (state.mode !== 'value') {
    control = (
      <span className="db-newrow-placeholder">{state.mode === 'null' ? 'NULL' : defaultLabel}</span>
    )
  } else if (kind === 'bool') {
    control = (
      <Select
        id={id}
        aria-label={col.name}
        value={state.text === 'true' || state.text === 'false' ? state.text : undefined}
        placeholder="true or false"
        options={[
          { value: 'true', label: 'true' },
          { value: 'false', label: 'false' },
        ]}
        disabled={disabled}
        onChange={setText}
      />
    )
  } else if (kind === 'json' || kind === 'longtext') {
    control = (
      <textarea
        id={id}
        aria-label={col.name}
        className="db-newrow-textarea"
        rows={kind === 'json' ? 4 : 3}
        value={state.text}
        spellCheck={kind !== 'json'}
        placeholder={kind === 'json' ? '{"key": "value"}' : ''}
        disabled={disabled}
        onChange={(event) => setText(event.target.value)}
      />
    )
  } else {
    const type =
      kind === 'integer' || kind === 'number' || kind === 'decimal'
        ? 'text'
        : kind === 'date'
          ? 'date'
          : kind === 'datetime'
            ? 'datetime-local'
            : kind === 'time'
              ? 'time'
              : 'text'
    control = (
      <Input
        id={id}
        aria-label={col.name}
        className="db-newrow-input"
        type={type}
        inputMode={kind === 'integer' ? 'numeric' : kind === 'number' || kind === 'decimal' ? 'decimal' : undefined}
        step={kind === 'datetime' || kind === 'time' ? 1 : undefined}
        value={state.text}
        placeholder={kind === 'binary' ? 'base64' : ''}
        spellCheck={false}
        autoComplete="off"
        disabled={disabled}
        onChange={setText}
      />
    )
  }

  return (
    <div className={`db-newrow-field${error ? ' invalid' : ''}`}>
      <label className="db-newrow-label" htmlFor={id}>
        <span className="name">{col.name}</span>
        {col.pk ? <KeyRound size={16} aria-label="primary key" /> : null}
        <span className="type">
          {col.type}
          {col.nullable ? '' : ' · required'}
        </span>
        {col.fk ? (
          <span className="fk">
            <Link2 size={16} aria-hidden />
            {fkTable(col.fk)}.{col.fk.column}
          </span>
        ) : null}
      </label>
      <div className="db-newrow-control">
        {modes.length > 1 ? (
          <Select<FieldMode>
            appearance="inline"
            aria-label={`${col.name} value source`}
            value={state.mode}
            options={modes.map((m) => ({ value: m, label: MODE_LABEL[m] }))}
            disabled={disabled}
            onChange={(mode) => onChange({ ...state, mode })}
          />
        ) : null}
        <span className="db-newrow-value">{control}</span>
        {onPick ? (
          <Button type="button" variant="ghost" size="sm" disabled={disabled} onClick={onPick}>
            Select row
          </Button>
        ) : null}
      </div>
      {error ? <span className="db-newrow-error">{error}</span> : null}
    </div>
  )
}

/** The referenced table, filterable and paged; a click picks a row's key. */
function RowPicker({
  host,
  db,
  table,
  column,
  dialect,
  onBack,
  onPick,
}: {
  host: Host
  db: string
  table: string
  column: string
  dialect?: SqlDialect
  onBack: () => void
  onPick: (value: unknown) => void
}) {
  const [page, setPage] = useState(0)
  const [filters, setFilters] = useState<FilterSpec[]>([])
  const applied = useMemo(() => filters.filter((f) => !f.disabled && isComplete(f)), [filters])
  const appliedKey = JSON.stringify(applied)
  const pageFetcher = useCallback(
    () =>
      browseTableRef(host, db, table, {
        page,
        pageSize: PICKER_PAGE,
        filters: JSON.parse(appliedKey) as FilterSpec[],
        includeTotal: false,
      }),
    [host, db, table, page, appliedKey],
  )
  const columnsFetcher = useCallback(() => tableColumns(host, db, 'unknown', table), [host, db, table])
  const pageRead = useDatabaseRead(true, pageFetcher)
  const columnsRead = useDatabaseRead(true, columnsFetcher)
  const rows = pageRead.data?.rows ?? []
  const pk = (columnsRead.data ?? []).filter((c) => c.pk).map((c) => c.name)

  return (
    <div className="db-newrow-picker">
      <div className="db-newrow-picker-bar">
        <Button type="button" variant="ghost" size="sm" onClick={onBack}>
          <ChevronLeft size={16} aria-hidden />
          Back to the form
        </Button>
      </div>
      <FilterBar
        columns={columnsRead.data ?? []}
        filters={filters}
        dialect={dialect}
        onChange={(next) => {
          setFilters(next)
          setPage(0)
        }}
      />
      <div className="db-newrow-picker-grid">
        {pageRead.error ? (
          <StatusPanel
            variant="alert"
            icon={<CircleAlert size={18} />}
            headline="Could not read the table"
            detail={pageRead.error}
          />
        ) : (
          <ResultGrid
            columns={pageRead.data?.columns}
            rows={rows}
            primaryKeys={pk}
            onRowClick={(i) => {
              const row = rows[i]
              if (row) onPick(row[column])
            }}
            stickyHeader
            hideFooter
          />
        )}
      </div>
      <div className="db-newrow-picker-foot">
        <span className="db-newrow-note">
          {pageRead.loading ? 'Loading…' : `Page ${page + 1} · click a row to use its ${column}`}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={page === 0 || pageRead.loading}
          onClick={() => setPage((p) => Math.max(0, p - 1))}
          aria-label="previous page"
        >
          <ChevronLeft size={16} aria-hidden />
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={!pageRead.data?.has_more || pageRead.loading}
          onClick={() => setPage((p) => p + 1)}
          aria-label="next page"
        >
          <ChevronRight size={16} aria-hidden />
        </Button>
      </div>
    </div>
  )
}

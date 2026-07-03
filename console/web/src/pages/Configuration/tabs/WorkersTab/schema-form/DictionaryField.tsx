import { useEffect, useMemo, useRef, useState } from 'react'
import { Input } from '@/components/ui/Input'
import { cn } from '@/lib/utils'
import type { JsonSchema, JsonValue } from '../api'
import { wt } from '../typography'
import { FieldDispatch, type FieldProps } from './FieldDispatch'
import { errorForField, FieldShell } from './FieldShell'
import { pathToDomId } from './path'
import { DeleteButton, DragHandle } from './RowChrome'
import { isInlineLeaf, schemaDefault } from './ref-resolver'
import { moveItem } from './reorder'
import { useDragReorder } from './useDragReorder'

interface DictionaryRow {
  /** Stable identity across renames + reorders. Used as the React key so the
   * row's DOM survives key edits without remounting the input (would lose
   * focus). */
  id: string
  /** Currently-typed key. Lives in row state because validation (uniqueness)
   * happens before we commit to the canonical value object. */
  key: string
}

let _rowIdCounter = 0
function makeRowId(): string {
  _rowIdCounter += 1
  return `row-${_rowIdCounter}`
}

/**
 * Env-file style key/value editor for `type: object` schemas that only
 * declare `additionalProperties` (no concrete `properties`). Each entry is
 * a row: a drag handle, the key input, the value control (inline when the
 * value is a primitive — the common `${VAR}`-style env case — or stacked
 * below for object/array values), and a trash button. An "+ add entry"
 * button appends a fresh row.
 *
 * Order is operator-controlled: rows can be dragged (or arrow-keyed) to
 * reorder, and the committed value object is rebuilt in row order. Object
 * key insertion order is preserved through the save round-trip.
 *
 * Uniqueness is enforced visually (a warn tint + inline marker on the
 * offending row) and structurally — duplicate or empty keys never commit
 * upward to the value object, so a typo doesn't drop data. `minProperties`
 * disables delete when removing would drop below the schema minimum.
 */
export function DictionaryField(props: FieldProps) {
  const { label, schema, value, onChange, rootSchema, required, path, errors } =
    props
  const description =
    typeof schema.description === 'string' ? schema.description : undefined
  const minProperties =
    typeof schema.minProperties === 'number' ? schema.minProperties : 0

  const obj = isPlainObject(value) ? value : {}
  const valueSchema = useMemo<JsonSchema>(() => {
    if (
      schema.additionalProperties &&
      typeof schema.additionalProperties === 'object' &&
      !Array.isArray(schema.additionalProperties)
    ) {
      return schema.additionalProperties as JsonSchema
    }
    return {}
  }, [schema.additionalProperties])
  const inlineValue = useMemo(
    () => isInlineLeaf(valueSchema, rootSchema),
    [valueSchema, rootSchema],
  )

  const [rows, setRows] = useState<DictionaryRow[]>(() =>
    Object.keys(obj).map((key) => ({ id: makeRowId(), key })),
  )
  const [lastAddedId, setLastAddedId] = useState<string | null>(null)

  const lastSyncedKeysRef = useRef<string>(
    rows
      .map((r) => r.key)
      .sort()
      .join('\u0000'),
  )
  useEffect(() => {
    const objKeys = Object.keys(obj).sort().join('\u0000')
    const ourValidKeys = rows
      .map((r) => r.key)
      .filter((k) => k.length > 0)
      .sort()
      .join('\u0000')
    if (objKeys !== lastSyncedKeysRef.current && objKeys !== ourValidKeys) {
      setRows(Object.keys(obj).map((key) => ({ id: makeRowId(), key })))
      lastSyncedKeysRef.current = objKeys
    }
  }, [obj, rows])

  const keyCounts = new Map<string, number>()
  for (const r of rows) {
    if (r.key.length === 0) continue
    keyCounts.set(r.key, (keyCounts.get(r.key) ?? 0) + 1)
  }

  function commitRows(nextRows: DictionaryRow[]) {
    setRows(nextRows)
    const counts = new Map<string, number>()
    for (const r of nextRows) {
      if (r.key.length === 0) continue
      counts.set(r.key, (counts.get(r.key) ?? 0) + 1)
    }
    const allUnique = [...counts.values()].every((c) => c === 1)
    const allNonEmpty = nextRows.every((r) => r.key.length > 0)
    if (allUnique && allNonEmpty) {
      const next: { [key: string]: JsonValue } = {}
      for (const r of nextRows) {
        next[r.key] = obj[r.key] ?? schemaDefault(valueSchema)
      }
      onChange(next)
      // Order-independent: reordering keeps the same key set, so external
      // sync won't fight a drag.
      lastSyncedKeysRef.current = nextRows
        .map((r) => r.key)
        .sort()
        .join('\u0000')
    }
  }

  function handleKeyChange(rowId: string, nextKey: string) {
    commitRows(rows.map((r) => (r.id === rowId ? { ...r, key: nextKey } : r)))
  }

  function handleValueChange(row: DictionaryRow, nextValue: JsonValue) {
    if (row.key.length === 0) return
    if ((keyCounts.get(row.key) ?? 0) > 1) return
    onChange({ ...obj, [row.key]: nextValue })
  }

  function handleAdd() {
    const existing = new Set(rows.map((r) => r.key))
    const base = 'new_entry'
    let candidate = base
    let i = 1
    while (existing.has(candidate)) {
      candidate = `${base}_${i++}`
    }
    const id = makeRowId()
    setLastAddedId(id)
    commitRows([...rows, { id, key: candidate }])
  }

  function handleDelete(rowId: string) {
    commitRows(rows.filter((r) => r.id !== rowId))
  }

  function handleReorder(from: number, to: number) {
    commitRows(moveItem(rows, from, to))
  }

  const canDelete = rows.length > minProperties
  const dnd = useDragReorder(rows.length, handleReorder)

  return (
    <section className="space-y-3">
      <FieldShell
        label={label}
        description={description}
        required={required}
        errorMessage={errorForField(props)}
        anchorId={pathToDomId(path)}
      >
        {() => (
          <div className={cn(wt.caption, 'text-ink-faint')}>
            {rows.length === 0
              ? 'no entries · add one below'
              : `${rows.length} entr${rows.length === 1 ? 'y' : 'ies'}`}
            {minProperties > 0 ? ` · min ${minProperties}` : null}
          </div>
        )}
      </FieldShell>

      {rows.length > 0 ? (
        <ul className="space-y-2">
          {rows.map((row, idx) => {
            const duplicate = (keyCounts.get(row.key) ?? 0) > 1
            const empty = row.key.length === 0
            const flagged = duplicate || empty
            const isDragging = dnd.dragIndex === idx
            const isOver = dnd.overIndex === idx && dnd.dragIndex !== idx
            const valueField = (
              <FieldDispatch
                label={row.key || 'value'}
                schema={valueSchema}
                value={obj[row.key]}
                onChange={(next) => handleValueChange(row, next)}
                rootSchema={rootSchema}
                path={[...path, row.key]}
                errors={errors}
                hideLabel
              />
            )
            return (
              <li
                key={row.id}
                ref={dnd.setRowRef(idx)}
                {...dnd.rowProps(idx)}
                className={cn(
                  inlineValue
                    ? 'space-y-1'
                    : 'border border-rule p-2 space-y-2',
                  isOver && 'bg-rule/40',
                  isDragging && 'opacity-40',
                )}
              >
                <div className="flex items-center gap-2">
                  <DragHandle
                    label={`reorder ${row.key || '(empty)'}`}
                    handle={dnd.handleProps(idx)}
                  />
                  <div
                    className={cn(
                      inlineValue ? 'basis-2/5 min-w-0' : 'flex-1 min-w-0',
                    )}
                  >
                    <Input
                      value={row.key}
                      onChange={(next) => handleKeyChange(row.id, next)}
                      preserveCase
                      autoFocus={row.id === lastAddedId}
                      placeholder="entry_name"
                      aria-label="entry key"
                      className={cn(flagged && 'border-warn', wt.control)}
                    />
                  </div>
                  {inlineValue ? (
                    <div className="flex-1 min-w-0">{valueField}</div>
                  ) : null}
                  <DeleteButton
                    label={`delete ${row.key || '(empty)'}`}
                    onClick={() => handleDelete(row.id)}
                    disabled={!canDelete}
                  />
                </div>
                {flagged ? (
                  <p className={cn(wt.micro, 'text-warn pl-7')}>
                    {empty ? 'key cannot be empty' : 'duplicate key'}
                  </p>
                ) : null}
                {!inlineValue ? (
                  <div className="border-l border-rule pl-3 ml-[10px]">
                    {valueField}
                  </div>
                ) : null}
              </li>
            )
          })}
        </ul>
      ) : null}

      <button
        type="button"
        onClick={handleAdd}
        className={cn(
          wt.bodySm,
          'text-ink-faint hover:text-ink',
          'border border-rule hover:border-ink px-3 h-8 transition-colors',
          'lowercase tracking-[0.04em]',
        )}
      >
        + add entry
      </button>
    </section>
  )
}

function isPlainObject(
  value: JsonValue | undefined,
): value is { [key: string]: JsonValue } {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

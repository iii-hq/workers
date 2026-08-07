import { useEffect, useMemo, useState } from 'react'
import { moveItem } from '@/lib/reorder'
import { cn } from '@/lib/utils'
import type { JsonSchema, JsonValue } from '../api'
import { wt } from '../typography'
import { FieldDispatch, type FieldProps } from './FieldDispatch'
import { errorForField, FieldShell } from './FieldShell'
import { pathToDomId } from './path'
import { DeleteButton, DragHandle } from './RowChrome'
import { isInlineLeaf, schemaDefault } from './ref-resolver'
import { useDragReorder } from './useDragReorder'

let _itemIdCounter = 0
function makeItemId(): string {
  _itemIdCounter += 1
  return `item-${_itemIdCounter}`
}

/**
 * `type: array` editor.
 *
 * Primitive items (string / number / boolean / enum) render as compact
 * single-control rows — a drag handle, the value control, and a trash
 * button — the same env-file vocabulary as `DictionaryField`. Complex
 * items (objects, arrays, oneOf) keep a bordered box with a `#N` header so
 * their nested structure stays legible.
 *
 * Rows carry a stable per-item id (not the array index) as their React
 * key, so dragging a row moves its actual DOM — including any live Lexical
 * editor — to the new slot instead of leaving it in place and re-seeding.
 * The ids track add / delete / reorder in lockstep and re-sync when the
 * array length changes externally (reset, mutation).
 *
 * `minItems` disables the per-row delete when removing would drop below
 * the schema minimum. `maxItems` is not enforced inline — the server
 * validates on save and a hard cap would be more annoying than the rare
 * reject it prevents.
 */
export function ArrayField(props: FieldProps) {
  const { label, schema, value, onChange, rootSchema, required, path, errors } =
    props
  const description =
    typeof schema.description === 'string' ? schema.description : undefined
  const minItems = typeof schema.minItems === 'number' ? schema.minItems : 0

  const items = Array.isArray(value) ? value : []

  const itemsSchema = useMemo<JsonSchema>(() => {
    if (
      schema.items &&
      typeof schema.items === 'object' &&
      !Array.isArray(schema.items)
    ) {
      return schema.items as JsonSchema
    }
    return {}
  }, [schema.items])
  const inlineItem = useMemo(
    () => isInlineLeaf(itemsSchema, rootSchema),
    [itemsSchema, rootSchema],
  )

  // Stable React keys for each row. Internal handlers keep them aligned;
  // a length mismatch means the value was replaced from outside (reset /
  // cache re-seed), so we regenerate.
  const itemCount = items.length
  const [ids, setIds] = useState<string[]>(() =>
    Array.from({ length: itemCount }, makeItemId),
  )
  useEffect(() => {
    setIds((cur) =>
      cur.length === itemCount
        ? cur
        : Array.from({ length: itemCount }, makeItemId),
    )
  }, [itemCount])

  function handleItemChange(idx: number, next: JsonValue) {
    const copy = items.slice()
    copy[idx] = next
    onChange(copy)
  }

  function handleAdd() {
    onChange([...items, schemaDefault(itemsSchema)])
    setIds((cur) => [...cur, makeItemId()])
  }

  function handleDelete(idx: number) {
    onChange(items.filter((_, i) => i !== idx))
    setIds((cur) => cur.filter((_, i) => i !== idx))
  }

  function handleReorder(from: number, to: number) {
    onChange(moveItem(items, from, to))
    setIds((cur) => moveItem(cur, from, to))
  }

  const canDelete = items.length > minItems
  const dnd = useDragReorder(items.length, handleReorder)

  return (
    <section className="space-y-2">
      <FieldShell
        label={label}
        description={description}
        required={required}
        errorMessage={errorForField(props)}
        anchorId={pathToDomId(path)}
      >
        {() => (
          <div className={cn(wt.caption, 'text-ink-faint')}>
            {items.length === 0
              ? 'empty · add an item below'
              : `${items.length} item${items.length === 1 ? '' : 's'}`}
            {minItems > 0 ? ` · min ${minItems}` : null}
          </div>
        )}
      </FieldShell>

      <ul className={inlineItem ? 'space-y-2' : 'space-y-3'}>
        {items.map((item, idx) => {
          const isDragging = dnd.dragIndex === idx
          const isOver = dnd.overIndex === idx && dnd.dragIndex !== idx
          const itemField = (
            <FieldDispatch
              label={inlineItem ? `item ${idx + 1}` : 'value'}
              schema={itemsSchema}
              value={item}
              onChange={(next) => handleItemChange(idx, next)}
              rootSchema={rootSchema}
              path={[...path, idx]}
              errors={errors}
              hideLabel={inlineItem}
            />
          )
          return (
            <li
              key={ids[idx] ?? idx}
              ref={dnd.setRowRef(idx)}
              {...dnd.rowProps(idx)}
              className={cn(
                inlineItem
                  ? 'flex items-center gap-2'
                  : 'border border-rule p-3 space-y-3',
                isOver && 'bg-surface-hover',
                isDragging && 'opacity-40',
              )}
            >
              {inlineItem ? (
                <>
                  <DragHandle
                    label={`reorder item ${idx + 1}`}
                    handle={dnd.handleProps(idx)}
                  />
                  <div className="flex-1 min-w-0">{itemField}</div>
                  <DeleteButton
                    label={`delete item ${idx + 1}`}
                    onClick={() => handleDelete(idx)}
                    disabled={!canDelete}
                  />
                </>
              ) : (
                <>
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-1">
                      <DragHandle
                        label={`reorder item ${idx + 1}`}
                        handle={dnd.handleProps(idx)}
                      />
                      <span
                        className={cn(
                          wt.micro,
                          'uppercase tracking-[0.06em] text-ink-faint',
                        )}
                      >
                        #{idx + 1}
                      </span>
                    </div>
                    <DeleteButton
                      label={`delete item ${idx + 1}`}
                      onClick={() => handleDelete(idx)}
                      disabled={!canDelete}
                    />
                  </div>
                  <div className="border-l border-rule pl-3">{itemField}</div>
                </>
              )}
            </li>
          )
        })}
      </ul>

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
        + add item
      </button>
    </section>
  )
}

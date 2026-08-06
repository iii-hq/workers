/**
 * Filters as chips, compiled server-side.
 *
 * Each chip is segmented — column, operator, value — and every segment is its
 * own control, so refining one part of a condition does not mean retyping the
 * rest. Two behaviours are worth calling out because both are easy to omit and
 * both are what make a filter bar usable rather than merely present:
 *
 * A chip can be **switched off** instead of deleted. Narrowing a query is
 * iterative, and "does this condition matter?" is the question being asked
 * most often; answering it by deleting and rebuilding loses the work.
 *
 * An **incomplete chip is a draft** and is never sent. The worker would reject
 * it, and a round trip that ends in an error would make typing feel broken.
 * Drafts read as dashed, the repo's convention for content that is not yet
 * real.
 *
 * Filters stack with AND. `in` exists precisely because OR does not: "status
 * is one of open, held" cannot be expressed as two AND'd equalities.
 */

import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Input,
} from '@iii-dev/console-ui'
import { useState } from 'react'
import { type FilterOp, type FilterSpec, isComplete, OP_LABEL, opsFor, SET_OPS, typeCategory } from '../lib/rpc'
import { Eye, EyeOff, Plus, X } from './icons'

/** The minimum a column must expose to be filterable. */
export interface FilterColumn {
  name: string
  type: string
}

export function FilterBar({
  columns,
  filters,
  onChange,
}: {
  columns: FilterColumn[]
  filters: FilterSpec[]
  onChange: (next: FilterSpec[]) => void
}) {
  const replace = (i: number, f: FilterSpec) => onChange(filters.map((x, j) => (j === i ? f : x)))
  const remove = (i: number) => onChange(filters.filter((_, j) => j !== i))

  const add = () => {
    const first = columns[0]
    if (!first) return
    onChange([...filters, { column: first.name, op: opsFor(typeCategory(first.type))[0] }])
  }

  const active = filters.filter((f) => !f.disabled && isComplete(f)).length
  const drafts = filters.filter((f) => !isComplete(f)).length

  return (
    <div className="db-filters">
      {filters.map((f, i) => (
        <Chip
          // Index-keyed on purpose: chips are positional and editing one in
          // place must not remount it and drop focus mid-edit.
          key={i}
          filter={f}
          columns={columns}
          onChange={(next) => replace(i, next)}
          onRemove={() => remove(i)}
        />
      ))}

      <Button variant="ghost" size="sm" onClick={add} disabled={columns.length === 0}>
        <Plus size={12} aria-hidden />
        filter
      </Button>

      {filters.length > 0 ? (
        <>
          <span className="db-filters-count">
            {active} applied
            {drafts > 0 ? ` · ${drafts} draft` : ''}
          </span>
          <button type="button" className="db-filters-clear" onClick={() => onChange([])}>
            clear all
          </button>
        </>
      ) : null}
    </div>
  )
}

function Chip({
  filter,
  columns,
  onChange,
  onRemove,
}: {
  filter: FilterSpec
  columns: FilterColumn[]
  onChange: (next: FilterSpec) => void
  onRemove: () => void
}) {
  const col = columns.find((c) => c.name === filter.column)
  const ops = opsFor(typeCategory(col?.type))
  const draft = !isComplete(filter)
  const isSet = SET_OPS.has(filter.op)
  const takesValue = !NO_VALUE.has(filter.op)

  const setColumn = (name: string) => {
    const next = columns.find((c) => c.name === name)
    const allowed = opsFor(typeCategory(next?.type))
    // Keep the operator when the new column still supports it; otherwise fall
    // back rather than leaving a chip that cannot compile.
    onChange({
      ...filter,
      column: name,
      op: allowed.includes(filter.op) ? filter.op : allowed[0],
    })
  }

  const setOp = (op: FilterOp) => {
    // Moving between shapes must not carry stale operands across, or a chip
    // reads as complete while holding a value its operator ignores.
    const carry: FilterSpec = { ...filter, op }
    if (SET_OPS.has(op)) {
      carry.values = filter.values ?? (filter.value != null ? [filter.value] : [])
      carry.value = undefined
      carry.value2 = undefined
    } else {
      carry.value = filter.value ?? filter.values?.[0]
      carry.values = undefined
      if (op !== 'between') carry.value2 = undefined
    }
    onChange(carry)
  }

  return (
    <span className={`db-chip${draft ? ' draft' : ''}${filter.disabled ? ' off' : ''}`}>
      <button
        type="button"
        className="db-chip-seg toggle"
        aria-pressed={!filter.disabled}
        title={filter.disabled ? 'apply this filter' : 'switch this filter off without removing it'}
        onClick={() => onChange({ ...filter, disabled: !filter.disabled })}
      >
        {filter.disabled ? <EyeOff size={11} aria-hidden /> : <Eye size={11} aria-hidden />}
      </button>

      <Picker
        label={filter.column || 'column'}
        className="col"
        items={columns.map((c) => ({ value: c.name, label: c.name, hint: c.type.toLowerCase() }))}
        onPick={setColumn}
      />

      <Picker
        label={OP_LABEL[filter.op]}
        className="op"
        items={ops.map((o) => ({ value: o, label: OP_LABEL[o], hint: o }))}
        onPick={(v) => setOp(v as FilterOp)}
      />

      {takesValue ? (
        isSet ? (
          <SetValue
            values={(filter.values ?? []) as unknown[]}
            onChange={(values) => onChange({ ...filter, values })}
          />
        ) : (
          <>
            <ValueInput value={filter.value} onChange={(value) => onChange({ ...filter, value })} label="value" />
            {filter.op === 'between' ? (
              <ValueInput
                value={filter.value2}
                onChange={(value2) => onChange({ ...filter, value2 })}
                label="upper bound"
              />
            ) : null}
          </>
        )
      ) : null}

      <button type="button" className="db-chip-seg remove" onClick={onRemove} aria-label="remove filter">
        <X size={11} aria-hidden />
      </button>
    </span>
  )
}

const NO_VALUE: ReadonlySet<FilterOp> = new Set(['is_true', 'is_false', 'is_null', 'is_not_null', 'is_empty'])

function Picker({
  label,
  className,
  items,
  onPick,
}: {
  label: string
  className: string
  items: { value: string; label: string; hint?: string }[]
  onPick: (value: string) => void
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button type="button" className={`db-chip-seg ${className}`}>
          {label}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        {items.map((it) => (
          <DropdownMenuItem key={it.value} onClick={() => onPick(it.value)}>
            <span className="db-pick-label">{it.label}</span>
            {it.hint ? <span className="db-pick-hint">{it.hint}</span> : null}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function ValueInput({ value, onChange, label }: { value: unknown; onChange: (v: string) => void; label: string }) {
  return (
    <span className="db-chip-seg val">
      <Input
        value={value == null ? '' : String(value)}
        onChange={onChange}
        placeholder={label}
        preserveCase
        aria-label={label}
        className="db-chip-input"
      />
    </span>
  )
}

/** Comma-separated entry for `in` / `not_in`. */
function SetValue({ values, onChange }: { values: unknown[]; onChange: (v: unknown[]) => void }) {
  const [text, setText] = useState(values.map((v) => String(v)).join(', '))
  return (
    <span className="db-chip-seg val set">
      <Input
        value={text}
        onChange={(next) => {
          setText(next)
          onChange(
            next
              .split(',')
              .map((s) => s.trim())
              .filter((s) => s !== ''),
          )
        }}
        placeholder="a, b, c"
        preserveCase
        aria-label="values, comma separated"
        className="db-chip-input wide"
      />
    </span>
  )
}

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
 * is one of open, held" cannot be expressed as two AND'd equalities — and
 * where a chip cannot say it at all, a **raw SQL chip** can: one highlighted
 * line holding a boolean expression, applied on Enter (a half-typed condition
 * is not worth a failing read per keystroke), ANDed in parentheses by the
 * worker inside a read-only transaction.
 *
 * While a value is being typed, the column's most frequent values
 * (`database::distinctValues`, narrowed by what is typed) line up under the
 * bar; one click fills the value, or adds it to an `is one of` list.
 */

import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Input,
} from '@iii-dev/console-ui'
import { Code, Eye, EyeOff, Plus, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { cellText } from '../lib/grid-cursor'
import {
  type FilterOp,
  type FilterSpec,
  isComplete,
  isRawFilter,
  OP_LABEL,
  opsFor,
  rawFilter,
  SET_OPS,
  typeCategory,
} from '../lib/rpc'
import type { SqlDialect } from '../lib/sql'
import { RawSqlInput } from './RawSqlInput'

/** The minimum a column must expose to be filterable. */
export interface FilterColumn {
  name: string
  type: string
}

/** One of a column's values, with how many rows hold it. */
export interface ValueSuggestion {
  value: unknown
  count: number
}

export type SuggestFn = (column: string, search: string) => Promise<ValueSuggestion[]>

/** Which value input has the focus, and what it holds. */
interface SuggestTarget {
  index: number
  slot: 'value' | 'value2' | 'values'
  search: string
}

const SUGGEST_DEBOUNCE_MS = 160

export function FilterBar({
  columns,
  filters,
  onChange,
  allowRaw = false,
  dialect,
  suggest,
}: {
  columns: FilterColumn[]
  filters: FilterSpec[]
  onChange: (next: FilterSpec[]) => void
  /** The worker takes raw SQL conditions (`FilterSpec.raw`). */
  allowRaw?: boolean
  dialect?: SqlDialect
  /** Value suggestions for a column; absent when the worker cannot give them. */
  suggest?: SuggestFn
}) {
  const replace = (i: number, f: FilterSpec) => onChange(filters.map((x, j) => (j === i ? f : x)))
  const remove = (i: number) => {
    setTarget(null)
    onChange(filters.filter((_, j) => j !== i))
  }

  const [target, setTarget] = useState<SuggestTarget | null>(null)
  const [suggestions, setSuggestions] = useState<ValueSuggestion[]>([])
  const targetFilter = target ? filters[target.index] : undefined
  const targetColumn = targetFilter && !isRawFilter(targetFilter) ? targetFilter.column : ''

  useEffect(() => {
    if (!suggest || !target || !targetColumn) {
      setSuggestions([])
      return
    }
    let alive = true
    const timer = setTimeout(() => {
      suggest(targetColumn, target.search)
        .then((values) => {
          if (alive) setSuggestions(values.filter((v) => v.value !== null && v.value !== undefined))
        })
        .catch(() => {
          if (alive) setSuggestions([])
        })
    }, SUGGEST_DEBOUNCE_MS)
    return () => {
      alive = false
      clearTimeout(timer)
    }
  }, [suggest, target, targetColumn])

  const pickSuggestion = (value: unknown) => {
    if (!target || !targetFilter) return
    if (target.slot === 'values') {
      const current = targetFilter.values ?? []
      if (!current.some((v) => cellText(v) === cellText(value))) {
        replace(target.index, { ...targetFilter, values: [...current, value] })
      }
    } else {
      replace(target.index, { ...targetFilter, [target.slot]: value })
    }
  }

  const add = () => {
    const first = columns[0]
    if (!first) return
    onChange([...filters, { column: first.name, op: opsFor(typeCategory(first.type))[0] }])
  }

  const active = filters.filter((f) => !f.disabled && isComplete(f)).length
  const drafts = filters.filter((f) => !isComplete(f)).length
  const focusValue = (index: number) => (slot: SuggestTarget['slot'], search: string) =>
    setTarget({ index, slot, search })

  return (
    <div className="db-filters">
      {filters.map((f, i) =>
        isRawFilter(f) ? (
          <RawChip
            key={i}
            filter={f}
            dialect={dialect}
            onChange={(next) => replace(i, next)}
            onRemove={() => remove(i)}
          />
        ) : (
          <Chip
            // Index-keyed on purpose: chips are positional and editing one in
            // place must not remount it and drop focus mid-edit.
            key={i}
            filter={f}
            columns={columns}
            onChange={(next) => replace(i, next)}
            onRemove={() => remove(i)}
            onValueFocus={suggest ? focusValue(i) : undefined}
            onValueBlur={() => setTarget(null)}
          />
        ),
      )}

      <Button variant="ghost" size="sm" onClick={add} disabled={columns.length === 0}>
        <Plus size={16} aria-hidden />
        Filter
      </Button>
      {allowRaw ? (
        <Button
          variant="ghost"
          size="sm"
          onClick={() => onChange([...filters, rawFilter()])}
          title="A SQL condition, e.g. total > 100 and status <> 'void'"
        >
          <Code size={16} aria-hidden />
          SQL
        </Button>
      ) : null}

      {filters.length > 0 ? (
        <>
          <span className="db-filters-count">
            {active} applied
            {drafts > 0 ? ` · ${drafts} draft` : ''}
          </span>
          <Button variant="ghost" size="sm" onClick={() => onChange([])}>
            Clear all
          </Button>
        </>
      ) : null}

      {target && suggestions.length > 0 ? (
        // A row of its own under the chips rather than a popover: a chip
        // clips its contents, and a strip stays readable at any pane width.
        // Picking keeps the focus in the value input (mousedown is swallowed).
        <div className="db-suggest" role="listbox" aria-label={`values of ${targetColumn}`}>
          <span className="db-suggest-label">{targetColumn}</span>
          {suggestions.map((s) => (
            <button
              key={cellText(s.value)}
              type="button"
              role="option"
              aria-selected={false}
              className="db-suggest-item"
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => pickSuggestion(s.value)}
              title={`${cellText(s.value)} — ${s.count.toLocaleString()} row${s.count === 1 ? '' : 's'}`}
            >
              <span className="v">{cellText(s.value)}</span>
              <span className="n">{s.count.toLocaleString()}</span>
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}

/**
 * A raw SQL condition. The line is a draft until Enter (or leaving the
 * field) commits it — every keystroke of a half-written expression would
 * otherwise be a read that fails. Escape restores the applied text.
 */
function RawChip({
  filter,
  dialect,
  onChange,
  onRemove,
}: {
  filter: FilterSpec
  dialect?: SqlDialect
  onChange: (next: FilterSpec) => void
  onRemove: () => void
}) {
  const applied = filter.raw ?? ''
  const [draft, setDraft] = useState(applied)
  const appliedRef = useRef(applied)
  useEffect(() => {
    // Another hand changed the applied text (Clear all, a reload): follow it.
    if (applied !== appliedRef.current) {
      appliedRef.current = applied
      setDraft(applied)
    }
  }, [applied])
  const commit = () => {
    if (draft === applied) return
    appliedRef.current = draft
    onChange({ ...filter, raw: draft })
  }
  const pending = draft !== applied
  return (
    <span className={`db-chip raw${!isComplete(filter) || pending ? ' draft' : ''}${filter.disabled ? ' off' : ''}`}>
      <button
        type="button"
        className="db-chip-seg toggle"
        aria-pressed={!filter.disabled}
        title={filter.disabled ? 'apply this filter' : 'switch this filter off without removing it'}
        onClick={() => onChange({ ...filter, disabled: !filter.disabled })}
      >
        {filter.disabled ? <EyeOff size={16} aria-hidden /> : <Eye size={16} aria-hidden />}
      </button>
      <span className="db-chip-seg op" title="a SQL boolean expression, ANDed with the other filters">
        SQL
      </span>
      <span className="db-chip-seg val">
        <RawSqlInput
          value={draft}
          onChange={setDraft}
          dialect={dialect}
          placeholder="total > 100 and status <> 'void'"
          aria-label="SQL condition — Enter applies"
          autoFocus={applied === ''}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault()
              commit()
            } else if (event.key === 'Escape') {
              event.preventDefault()
              setDraft(applied)
            }
          }}
          onBlur={commit}
        />
      </span>
      <button type="button" className="db-chip-seg remove" onClick={onRemove} aria-label="remove filter">
        <X size={16} aria-hidden />
      </button>
    </span>
  )
}

function Chip({
  filter,
  columns,
  onChange,
  onRemove,
  onValueFocus,
  onValueBlur,
}: {
  filter: FilterSpec
  columns: FilterColumn[]
  onChange: (next: FilterSpec) => void
  onRemove: () => void
  /** A value input took the focus (or its text changed while focused). */
  onValueFocus?: (slot: 'value' | 'value2' | 'values', search: string) => void
  onValueBlur?: () => void
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
        {filter.disabled ? <EyeOff size={16} aria-hidden /> : <Eye size={16} aria-hidden />}
      </button>

      <Picker
        label={filter.column || 'Column'}
        className="col"
        items={columns.map((c) => ({
          value: c.name,
          label: c.name,
          hint: c.type,
        }))}
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
            onFocusText={onValueFocus ? (search) => onValueFocus('values', search) : undefined}
            onBlur={onValueBlur}
          />
        ) : (
          <>
            <ValueInput
              value={filter.value}
              onChange={(value) => {
                onChange({ ...filter, value })
                onValueFocus?.('value', value)
              }}
              onFocus={() => onValueFocus?.('value', cellTextOrEmpty(filter.value))}
              onBlur={onValueBlur}
              label="Value"
            />
            {filter.op === 'between' ? (
              <ValueInput
                value={filter.value2}
                onChange={(value2) => {
                  onChange({ ...filter, value2 })
                  onValueFocus?.('value2', value2)
                }}
                onFocus={() => onValueFocus?.('value2', cellTextOrEmpty(filter.value2))}
                onBlur={onValueBlur}
                label="Upper bound"
              />
            ) : null}
          </>
        )
      ) : null}

      <button type="button" className="db-chip-seg remove" onClick={onRemove} aria-label="remove filter">
        <X size={16} aria-hidden />
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

function cellTextOrEmpty(value: unknown): string {
  return value == null ? '' : String(value)
}

function ValueInput({
  value,
  onChange,
  onFocus,
  onBlur,
  label,
}: {
  value: unknown
  onChange: (v: string) => void
  onFocus?: () => void
  onBlur?: () => void
  label: string
}) {
  return (
    <span className="db-chip-seg val">
      <Input
        value={cellTextOrEmpty(value)}
        onChange={onChange}
        onFocus={onFocus}
        onBlur={onBlur}
        placeholder={label}
        preserveCase
        aria-label={label}
        className="db-chip-input"
      />
    </span>
  )
}

/** The list a comma-separated `in` entry stands for. */
function parseSetText(text: string): string[] {
  return text
    .split(',')
    .map((s) => s.trim())
    .filter((s) => s !== '')
}

/** Comma-separated entry for `in` / `not_in`. */
function SetValue({
  values,
  onChange,
  onFocusText,
  onBlur,
}: {
  values: unknown[]
  onChange: (v: unknown[]) => void
  /** Focus, or typing while focused: the fragment after the last comma. */
  onFocusText?: (search: string) => void
  onBlur?: () => void
}) {
  const [text, setText] = useState(values.map((v) => String(v)).join(', '))
  // A value added from the suggestions arrives through `values`; the typed
  // text follows it rather than holding the list it had before.
  const shown = parseSetText(text).join('\u0000')
  const given = values.map((v) => String(v)).join('\u0000')
  // Re-sync only when the list itself changes.
  useEffect(() => {
    if (shown !== given) setText(values.map((v) => String(v)).join(', '))
  }, [given])
  const fragment = (t: string) => t.slice(t.lastIndexOf(',') + 1).trim()
  return (
    <span className="db-chip-seg val set">
      <Input
        value={text}
        onChange={(next) => {
          setText(next)
          onChange(parseSetText(next))
          onFocusText?.(fragment(next))
        }}
        onFocus={() => onFocusText?.(fragment(text))}
        onBlur={onBlur}
        placeholder="a, b, c"
        preserveCase
        aria-label="values, comma separated"
        className="db-chip-input wide"
      />
    </span>
  )
}

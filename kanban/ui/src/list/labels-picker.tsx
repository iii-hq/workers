import { Popover, PopoverContent, PopoverTrigger } from '@iii-dev/console-ui'
import { Check, Minus, Plus, Search } from 'lucide-react'
import { type KeyboardEvent, type ReactElement, useRef, useState } from 'react'
import { splitTrigger, swallow } from '../ui/option-menu'
import { toneOf } from '../ui/tone'

/** The server folds ASCII case only ("Bug" is "bug", "Ação" is not "AÇÃO"), so the picker must agree or a toggle would not stick. */
export const fold = (label: string) => label.replace(/[A-Z]/g, (char) => char.toLowerCase())

/**
 * Labels for a selection of tickets: every label the board uses, how many of the selected tickets carry it (all,
 * some, none), and a row that creates the label typed in the search. Checking a label adds it to every selected
 * ticket; unchecking removes it from all.
 */
export function LabelsPicker({
  labels,
  coverage,
  total,
  busy,
  onToggle,
  trigger,
}: {
  /** Every label on the board, most used first. */
  labels: string[]
  /** How many selected tickets carry a label (by its folded spelling). */
  coverage: (label: string) => number
  total: number
  busy: boolean
  onToggle: (label: string) => void
  trigger: ReactElement
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const list = useRef<HTMLDivElement>(null)
  const search = useRef<HTMLInputElement>(null)
  const q = query.trim()
  // "Bug" and "bug" are one label to the server: one row, under the spelling most tickets use.
  const spellings = new Map<string, string>()
  for (const label of labels) if (!spellings.has(fold(label))) spellings.set(fold(label), label)
  const existing = spellings.get(fold(q))
  const canCreate = q !== '' && !existing
  const rows = [...spellings.values()]
    .filter((label) => label.toLowerCase().includes(q.toLowerCase()))
    .map((label) => ({ label, have: coverage(label) }))
    .sort((a, b) => b.have - a.have)

  const commit = () => {
    if (busy || !q) return
    onToggle(existing ?? q)
    setQuery('')
  }

  const onKey = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const all = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('[data-picker-row]:not(:disabled)') ?? [])
    const at = all.indexOf(document.activeElement as HTMLButtonElement)
    const next = event.key === 'ArrowDown' ? Math.min(all.length - 1, at + 1) : at - 1
    if (next < 0) search.current?.focus()
    else all[next]?.focus()
  }

  const { render, children } = splitTrigger(trigger)
  return (
    <Popover
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) setQuery('')
      }}
      open={open}
    >
      <PopoverTrigger aria-label="Labels" disabled={busy} onClick={swallow} render={render}>
        {children}
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="kb-picker"
        initialFocus={search}
        onClick={swallow}
        onKeyDown={(event) => {
          swallow(event)
          onKey(event)
        }}
        side="top"
      >
        <div className="kb-picker__search">
          <Search size={16} />
          <input
            aria-label="Find or create a label"
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault()
                commit()
              }
            }}
            placeholder="Find or create a label…"
            ref={search}
            value={query}
          />
        </div>
        <div aria-label="Labels" aria-multiselectable className="kb-picker__list" ref={list} role="listbox">
          {rows.map(({ label, have }) => {
            const state = have === 0 ? 'none' : have === total ? 'all' : 'some'
            return (
              <button
                aria-selected={state === 'all'}
                className="kb-picker__row"
                data-checkbox="true"
                data-picker-row=""
                disabled={busy}
                key={label}
                onClick={() => onToggle(label)}
                role="option"
                type="button"
              >
                <span aria-hidden="true" className="kb-picker__check" data-on={state === 'none' ? undefined : 'true'}>
                  {state === 'all' ? <Check size={16} /> : state === 'some' ? <Minus size={16} /> : null}
                </span>
                <span aria-hidden="true" className="kb-label__dot" data-tone={toneOf(label)} />
                <span className="kb-picker__label">{label}</span>
                <span className="kb-picker__count">{state === 'none' ? '' : state === 'all' ? 'all' : `${have} of ${total}`}</span>
              </button>
            )
          })}
          {canCreate ? (
            <button className="kb-picker__row" data-picker-row="" disabled={busy} onClick={commit} role="option" aria-selected={false} type="button">
              <span aria-hidden="true" className="kb-picker__check">
                <Plus size={16} />
              </span>
              <span className="kb-picker__label">Create “{q}”</span>
            </button>
          ) : null}
          {rows.length === 0 && !canCreate ? <p className="kb-picker__empty">No labels on this board yet.</p> : null}
        </div>
        <p className="kb-picker__note">Checking a label adds it to every selected ticket; unchecking removes it from all.</p>
      </PopoverContent>
    </Popover>
  )
}

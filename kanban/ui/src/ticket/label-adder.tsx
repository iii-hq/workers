import { Popover, PopoverContent, PopoverTrigger, ShadcnButton } from '@iii-dev/console-ui'
import { Plus, Search } from 'lucide-react'
import { type KeyboardEvent, useMemo, useRef, useState } from 'react'
import { toneOf } from '../ui/tone'

/** The server compares labels with ASCII case ignored; so does the list. */
const asciiLower = (text: string) => text.replace(/[A-Z]/g, (char) => char.toLowerCase())

/**
 * "Add label": a popover with a search field over the board's labels the ticket does not carry yet, plus a
 * "Create label" row when the typed name is new. Enter takes the first row.
 */
export function LabelAdder({
  id,
  suggestions,
  own,
  disabled,
  onAdd,
}: {
  id?: string
  /** Every label on the board. */
  suggestions: string[]
  /** The ticket's labels: never offered again. */
  own: string[]
  disabled?: boolean
  onAdd: (label: string) => void
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const search = useRef<HTMLInputElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const typed = query.trim()
  const ownSet = useMemo(() => new Set(own.map(asciiLower)), [own])
  const rows = useMemo(() => {
    const needle = asciiLower(typed)
    return suggestions.filter((label) => !ownSet.has(asciiLower(label)) && asciiLower(label).includes(needle))
  }, [suggestions, ownSet, typed])
  const exact = suggestions.some((label) => asciiLower(label) === asciiLower(typed)) || ownSet.has(asciiLower(typed))
  const canCreate = typed.length > 0 && !exact

  const pick = (label: string) => {
    onAdd(label)
    setOpen(false)
    setQuery('')
  }
  const walk = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const all = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('button') ?? [])
    const at = all.indexOf(document.activeElement as HTMLButtonElement)
    const next = event.key === 'ArrowDown' ? Math.min(all.length - 1, at + 1) : at - 1
    if (next < 0) search.current?.focus()
    else all[next]?.focus()
  }

  return (
    <Popover
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) setQuery('')
      }}
      open={open}
    >
      <PopoverTrigger
        aria-label="Add label"
        disabled={disabled}
        id={id}
        render={<ShadcnButton className="kb-label-add" size="xs" type="button" variant="ghost" />}
      >
        <Plus size={16} />
        Add label
      </PopoverTrigger>
      <PopoverContent align="start" className="kb-picker" initialFocus={search} onKeyDown={walk}>
        <div className="kb-picker__search">
          <Search size={16} />
          <input
            aria-label="Find or create a label"
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== 'Enter') return
              event.preventDefault()
              const first = rows[0]
              if (first && asciiLower(first) === asciiLower(typed)) pick(first)
              else if (canCreate) pick(typed)
              else if (first) pick(first)
            }}
            placeholder="Find or create a label…"
            ref={search}
            value={query}
          />
        </div>
        <div aria-label="Labels" className="kb-picker__list" ref={list} role="listbox">
          {rows.map((label) => (
            <button
              aria-selected={false}
              className="kb-picker__row"
              key={label}
              onClick={() => pick(label)}
              role="option"
              type="button"
            >
              <span aria-hidden="true" className="kb-label__dot" data-tone={toneOf(label)} />
              <span className="kb-picker__label">{label}</span>
            </button>
          ))}
          {canCreate ? (
            <button aria-selected={false} className="kb-picker__row" onClick={() => pick(typed)} role="option" type="button">
              <Plus size={16} />
              <span className="kb-picker__label">Create label “{typed}”</span>
            </button>
          ) : null}
          {rows.length === 0 && !canCreate ? (
            <p className="kb-picker__empty">{typed ? 'The ticket already has it.' : 'Type to create a label.'}</p>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  )
}

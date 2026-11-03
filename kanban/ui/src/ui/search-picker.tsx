import { Popover, PopoverContent, PopoverTrigger } from '@iii-dev/console-ui'
import { Check, Search } from 'lucide-react'
import {
  Fragment,
  type KeyboardEvent,
  type ReactElement,
  type ReactNode,
  useId,
  useMemo,
  useRef,
  useState,
} from 'react'
import { splitTrigger, swallow } from './option-menu'

export type PickerOption = {
  value: string
  label: string
  icon?: ReactNode
  /** A second, quieter line or trailing word. */
  description?: string
  /** A count after the label (filter menus). */
  count?: number
  disabled?: boolean
  /** A small heading above this row, when it starts a group. */
  group?: string
}

type Common = {
  /** The heading, the field's accessible name. */
  label: string
  options: PickerOption[]
  trigger: ReactElement
  /** Shown above the list; the search field appears once there are more than a handful of rows. */
  searchPlaceholder?: string
  /** What an empty list says. */
  empty?: string
  align?: 'start' | 'center' | 'end'
  /** Extra controls above the list (the label match mode). */
  head?: ReactNode
  /** A line under the list (a hint, "First visit in this browser"). */
  note?: ReactNode
  disabled?: boolean
  onOpenChange?: (open: boolean) => void
}

type Single = Common & { multiple?: false; value: string | null; onChange: (value: string) => void }
type Multiple = Common & {
  multiple: true
  value: string[]
  onChange: (value: string) => void
  /** Clears every pick (shown in the footer). */
  onClear?: () => void
}

const SEARCH_FROM = 7

/**
 * A searchable picker: a shadcn popover with a search field and a list of rows (radio or checkbox). Used for
 * assignee, project, release, parent and the filter facets. Arrow keys walk the rows; typing filters them.
 */
export function SearchPicker(props: Single | Multiple) {
  const { label, options, trigger, searchPlaceholder, empty, align = 'start', head, note, disabled, onOpenChange } = props
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const list = useRef<HTMLDivElement>(null)
  const search = useRef<HTMLInputElement>(null)
  const listId = useId()
  const searchable = options.length > SEARCH_FROM || Boolean(searchPlaceholder && options.length > 3)
  const q = query.trim().toLowerCase()
  const shown = useMemo(
    () =>
      q
        ? options.filter(
            (option) => option.label.toLowerCase().includes(q) || option.description?.toLowerCase().includes(q),
          )
        : options,
    [options, q],
  )
  const picked = (value: string) => (props.multiple ? props.value.includes(value) : props.value === value)

  const change = (next: boolean) => {
    setOpen(next)
    if (!next) setQuery('')
    onOpenChange?.(next)
  }

  const rows = () => Array.from(list.current?.querySelectorAll<HTMLButtonElement>('[data-picker-row]:not(:disabled)') ?? [])
  const onListKey = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp' && event.key !== 'Home' && event.key !== 'End') return
    event.preventDefault()
    const all = rows()
    if (all.length === 0) return
    const at = all.indexOf(document.activeElement as HTMLButtonElement)
    const next =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? all.length - 1
          : event.key === 'ArrowDown'
            ? Math.min(all.length - 1, at + 1)
            : at <= 0
              ? -1
              : at - 1
    if (next < 0) search.current?.focus()
    else all[next]?.focus()
  }

  const { render, children } = splitTrigger(trigger)
  const choose = (value: string) => {
    props.onChange(value)
    if (!props.multiple) change(false)
  }

  return (
    <Popover onOpenChange={change} open={open}>
      <PopoverTrigger disabled={disabled} onClick={swallow} onKeyDown={swallow} render={render}>
        {children}
      </PopoverTrigger>
      <PopoverContent
        align={align}
        className="kb-picker"
        initialFocus={searchable ? search : true}
        onClick={swallow}
        onKeyDown={(event) => {
          swallow(event)
          onListKey(event)
        }}
      >
        {searchable ? (
          <div className="kb-picker__search">
            <Search size={16} />
            <input
              aria-controls={listId}
              aria-label={searchPlaceholder ?? `Find ${label.toLowerCase()}`}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault()
                  const first = shown.find((option) => !option.disabled)
                  if (first) choose(first.value)
                }
              }}
              placeholder={searchPlaceholder ?? `Find ${label.toLowerCase()}…`}
              ref={search}
              value={query}
            />
          </div>
        ) : (
          <p className="kb-picker__heading">{label}</p>
        )}
        {head ? <div className="kb-picker__head">{head}</div> : null}
        <div
          aria-label={label}
          aria-multiselectable={props.multiple ? true : undefined}
          className="kb-picker__list"
          id={listId}
          ref={list}
          role="listbox"
        >
          {shown.length === 0 ? (
            <p className="kb-picker__empty">{q ? `Nothing matches “${query.trim()}”.` : (empty ?? 'Nothing to pick yet.')}</p>
          ) : (
            shown.map((option, index) => {
              const on = picked(option.value)
              const heading = option.group && option.group !== shown[index - 1]?.group ? option.group : null
              return (
                <Fragment key={option.value}>
                  {heading ? <p className="kb-picker__group">{heading}</p> : null}
                  <button
                    aria-selected={on}
                    className="kb-picker__row"
                    data-checkbox={props.multiple ? 'true' : undefined}
                    data-picker-row=""
                    data-zero={option.count === 0 ? 'true' : undefined}
                    disabled={option.disabled}
                    onClick={() => choose(option.value)}
                    role="option"
                    type="button"
                  >
                    <span aria-hidden="true" className="kb-picker__check" data-on={on ? 'true' : undefined}>
                      {on ? <Check size={16} /> : null}
                    </span>
                    {option.icon}
                    <span className="kb-picker__label">{option.label}</span>
                    {option.description ? <span className="kb-picker__description">{option.description}</span> : null}
                    {option.count === undefined ? null : <span className="kb-picker__count">{option.count}</span>}
                  </button>
                </Fragment>
              )
            })
          )}
        </div>
        {props.multiple && props.onClear ? (
          <div className="kb-picker__foot">
            <span>{props.value.length === 0 ? 'None selected' : `${props.value.length} selected`}</span>
            <button
              className="kb-picker__clear"
              disabled={props.value.length === 0}
              onClick={() => props.onClear?.()}
              type="button"
            >
              Clear
            </button>
          </div>
        ) : null}
        {note ? <p className="kb-picker__note">{note}</p> : null}
      </PopoverContent>
    </Popover>
  )
}

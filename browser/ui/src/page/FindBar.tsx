/**
 * Find in page, under the address bar: the query, `3 of 12`, previous /
 * next, close. Typing searches after a short pause; Enter steps forward,
 * Shift+Enter back, Escape closes and clears the page's highlights.
 */

import { IconButton, Input } from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'
import { ChevronDown, ChevronUp, Search, X } from '../lib/icons'

export interface FindState {
  query: string
  count: number
  index: number
}

interface FindBarProps {
  state: FindState
  onQuery: (query: string) => void
  onNext: () => void
  onPrevious: () => void
  onClose: () => void
}

export function FindBar({
  state,
  onQuery,
  onNext,
  onPrevious,
  onClose,
}: FindBarProps) {
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])
  const empty = state.query.trim() === ''
  return (
    <search className="br-ui-findbar" aria-label="find in page">
      <Search size={16} aria-hidden className="br-ui-findbar-icon" />
      <Input
        ref={inputRef}
        value={state.query}
        onChange={onQuery}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            event.preventDefault()
            if (event.shiftKey) onPrevious()
            else onNext()
          } else if (event.key === 'Escape') {
            event.preventDefault()
            event.stopPropagation()
            onClose()
          }
        }}
        placeholder="Find in page"
        aria-label="find in page"
        preserveCase
        className="br-ui-findbar-input"
      />
      <span
        className="br-ui-findbar-count"
        aria-live="polite"
        data-empty={empty || undefined}
        data-none={(!empty && state.count === 0) || undefined}
      >
        {empty
          ? ''
          : state.count === 0
            ? 'No matches'
            : `${state.index} of ${state.count}`}
      </span>
      <IconButton
        label="previous match"
        onClick={onPrevious}
        disabled={state.count === 0}
      >
        <ChevronUp size={16} aria-hidden />
      </IconButton>
      <IconButton
        label="next match"
        onClick={onNext}
        disabled={state.count === 0}
      >
        <ChevronDown size={16} aria-hidden />
      </IconButton>
      <IconButton label="close find" onClick={onClose}>
        <X size={16} aria-hidden />
      </IconButton>
    </search>
  )
}

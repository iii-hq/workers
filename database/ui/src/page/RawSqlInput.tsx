/**
 * One line of SQL with highlighting — the raw filter's editor.
 *
 * Monaco is the editor for anything multi-line, but for a single condition
 * inside a chip it is a lot of machinery: this is a native input whose text
 * is transparent, laid exactly over a copy of the same text coloured by the
 * page's own lexer. The input keeps everything an input does (caret,
 * selection, IME, undo, autofill off); the copy only paints.
 */

import { forwardRef, type KeyboardEvent, useMemo, useRef } from 'react'
import { type SqlDialect, tokenizeSql } from '../lib/sql'

interface RawSqlInputProps {
  value: string
  onChange: (next: string) => void
  dialect?: SqlDialect
  placeholder?: string
  'aria-label': string
  className?: string
  onKeyDown?: (event: KeyboardEvent<HTMLInputElement>) => void
  onBlur?: () => void
  autoFocus?: boolean
}

export const RawSqlInput = forwardRef<HTMLInputElement, RawSqlInputProps>(function RawSqlInput(
  { value, onChange, dialect = 'unknown', placeholder, className, onKeyDown, onBlur, autoFocus, ...aria },
  ref,
) {
  const paintRef = useRef<HTMLSpanElement>(null)
  const tokens = useMemo(() => tokenizeSql(value, dialect), [value, dialect])
  return (
    <span className={`db-rawsql${className ? ` ${className}` : ''}`}>
      <span className="db-rawsql-paint" aria-hidden ref={paintRef}>
        {tokens.map((t) => (
          <span key={t.start} className={`db-tok-${t.kind}`}>
            {t.text}
          </span>
        ))}
      </span>
      <input
        ref={ref}
        className="db-rawsql-input"
        type="text"
        value={value}
        placeholder={placeholder}
        aria-label={aria['aria-label']}
        spellCheck={false}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
        // biome-ignore lint/a11y/noAutofocus: a raw chip is created to be typed into
        autoFocus={autoFocus}
        onChange={(event) => onChange(event.target.value.replace(/[\r\n]+/g, ' '))}
        onKeyDown={onKeyDown}
        onBlur={onBlur}
        // The painted copy follows the input's horizontal scroll, so a long
        // condition stays aligned with the caret.
        onScroll={(event) => {
          if (paintRef.current) paintRef.current.scrollLeft = event.currentTarget.scrollLeft
        }}
        onSelect={(event) => {
          if (paintRef.current) paintRef.current.scrollLeft = event.currentTarget.scrollLeft
        }}
      />
    </span>
  )
})

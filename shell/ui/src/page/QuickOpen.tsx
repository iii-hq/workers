/* Go to file (Ctrl+P): a palette-shaped overlay inside the shell page.
   One field, and under it the files of the pane's folder ranked by the
   worker's fuzzy scorer as you type — any characters of the path, in
   order, so `ptofolder` finds `path/to/folder`. Each row is the file's
   type icon, its name, and its folder in quieter ink, with the matched
   characters emphasised. Enter opens the row pinned in this pane; the
   empty query offers what this pane opened last. Built on the shared
   Dialog so the overlay, scrim, focus return and Escape are the console's;
   the rows are a listbox the field drives, which is why they are not the
   shared List (its rows take the focus themselves). */

import { Dialog, DialogContent, DialogTitle, type Host, StatusPanel } from '@iii-dev/console-ui'
import { Search } from 'lucide-react'
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import { coderSearch } from './coder'
import { FileTypeIcon } from './file-type-icon'
import {
  highlightSegments,
  normalizeQuickOpenQuery,
  type QuickOpenRow,
  quickOpenRow,
  stepQuickOpenIndex,
  toQuickOpenRows,
} from './quick-open'

const ROWS = 40
const DEBOUNCE_MS = 80

interface QuickOpenProps {
  host: Host
  /** The pane's browsed folder; `null` while none is chosen. */
  root: string | null
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Root-relative paths this pane opened, most recent first. */
  recent: readonly string[]
  onOpenFile: (relPath: string) => void
}

type Results =
  | { kind: 'idle' }
  | { kind: 'rows'; query: string; rows: QuickOpenRow[]; truncated: boolean }
  | { kind: 'error'; query: string; message: string }

function Highlighted({ text, indices, offset }: { text: string; indices: readonly number[] | null; offset: number }) {
  return (
    <>
      {highlightSegments(text, indices, offset).map((segment, index) =>
        segment.hit ? (
          <mark key={index} className="hit">
            {segment.text}
          </mark>
        ) : (
          <span key={index}>{segment.text}</span>
        ),
      )}
    </>
  )
}

/** Hands the focus back to whatever had it before the overlay opened, once
    the overlay is really gone: the shared dialog's own return can record
    its field instead of the opener when effects run twice, and a detached
    field cannot take the focus back. */
function FocusReturn({ target }: { target: React.MutableRefObject<HTMLElement | null> }) {
  useEffect(
    () => () => {
      const element = target.current
      if (!element) return
      window.setTimeout(() => {
        if (element.isConnected && document.activeElement === document.body) element.focus({ preventScroll: true })
      }, 0)
    },
    [target],
  )
  return null
}

export function QuickOpen({ host, root, open, onOpenChange, recent, onOpenFile }: QuickOpenProps) {
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<Results>({ kind: 'idle' })
  const [searching, setSearching] = useState(false)
  const [active, setActive] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const seqRef = useRef(0)
  const returnRef = useRef<HTMLElement | null>(null)
  const listId = useId()

  useLayoutEffect(() => {
    if (open) returnRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
  }, [open])

  const normalized = normalizeQuickOpenQuery(query)
  const folder = root?.split('/').filter(Boolean).at(-1) ?? null

  useEffect(() => {
    if (!open) return
    setQuery('')
    setResults({ kind: 'idle' })
    setSearching(false)
    setActive(0)
    seqRef.current += 1
  }, [open])

  useEffect(() => {
    if (!open || root === null) return
    if (normalized === '') {
      seqRef.current += 1
      setSearching(false)
      setResults({ kind: 'idle' })
      return
    }
    const seq = ++seqRef.current
    const timer = window.setTimeout(() => {
      setSearching(true)
      coderSearch(host, {
        query: normalized,
        regex: false,
        ignoreCase: true,
        path: root,
        searchContent: false,
        fuzzyPaths: true,
        respectGitignore: true,
        maxMatches: ROWS * 2,
      })
        .then((out) => {
          if (seq !== seqRef.current) return
          const rows = toQuickOpenRows(root, out.path_matches, normalized, ROWS)
          setResults({ kind: 'rows', query: normalized, rows, truncated: out.truncated || out.path_matches.length > ROWS })
          setActive(0)
        })
        .catch((err: unknown) => {
          if (seq !== seqRef.current) return
          setResults({ kind: 'error', query: normalized, message: errorMessage(err) })
        })
        .finally(() => {
          if (seq === seqRef.current) setSearching(false)
        })
    }, DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [open, root, normalized, host])

  const recentRows = useMemo(() => recent.map((rel) => quickOpenRow(rel, '')), [recent])
  const listing = normalized === ''
  const rows: QuickOpenRow[] = listing ? recentRows : results.kind === 'rows' ? results.rows : []

  useEffect(() => {
    const list = listRef.current
    if (!list || active < 0) return
    list.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' })
  }, [active, rows])

  const choose = useCallback(
    (row: QuickOpenRow | undefined) => {
      if (!row) return
      onOpenChange(false)
      onOpenFile(row.rel)
    },
    [onOpenChange, onOpenFile],
  )

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      setActive((current) => stepQuickOpenIndex(current, event.key === 'ArrowDown' ? 1 : -1, rows.length))
    } else if (event.key === 'Home' && rows.length > 0) {
      event.preventDefault()
      setActive(0)
    } else if (event.key === 'End' && rows.length > 0) {
      event.preventDefault()
      setActive(rows.length - 1)
    } else if (event.key === 'Enter') {
      event.preventDefault()
      choose(rows[active] ?? rows[0])
    }
  }

  const stale = results.kind === 'rows' && results.query !== normalized
  const activeId = rows[active] ? `${listId}-${active}` : undefined

  let note: React.ReactNode = null
  if (root === null) {
    note = <p className="shui-quick-open-note">Choose a folder first. The files of that folder open here.</p>
  } else if (listing && rows.length === 0) {
    note = <p className="shui-quick-open-note">Type any part of a path. Characters match in order, so a few letters from each folder are enough.</p>
  } else if (!listing && results.kind === 'error') {
    note = (
      <div className="shui-quick-open-status">
        <StatusPanel variant="alert" headline="The search failed" detail={results.message} />
      </div>
    )
  } else if (!listing && results.kind === 'rows' && rows.length === 0 && !searching) {
    note = (
      <p className="shui-quick-open-note">
        No file in {folder ?? 'this folder'} matches <span className="query">{query.trim()}</span>.
      </p>
    )
  }

  let summary: string
  if (root === null) summary = ''
  else if (listing) summary = rows.length > 0 ? 'Recently opened' : folder ?? ''
  else if (searching || stale || results.kind === 'idle') summary = 'Searching…'
  else if (results.kind === 'error') summary = ''
  else if (results.truncated) summary = `First ${rows.length} matches — keep typing to narrow`
  else summary = `${rows.length} ${rows.length === 1 ? 'file' : 'files'}`

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="shui-quick-open" aria-describedby={undefined} data-searching={searching ? 'true' : undefined}>
        <DialogTitle className="shui-quick-open-title">Go to file</DialogTitle>
        <FocusReturn target={returnRef} />
        <div className="shui-quick-open-search">
          <Search aria-hidden className="icon" />
          <input
            ref={inputRef}
            className="shui-quick-open-input"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value)
              setActive(0)
            }}
            onKeyDown={onKeyDown}
            placeholder={folder ? `Go to a file in ${folder}` : 'Go to file'}
            aria-label="Go to file"
            role="combobox"
            aria-expanded={rows.length > 0}
            aria-controls={listId}
            aria-activedescendant={activeId}
            aria-autocomplete="list"
            autoCapitalize="none"
            autoCorrect="off"
            autoComplete="off"
            spellCheck={false}
            enterKeyHint="go"
            data-autofocus
          />
        </div>
        <div ref={listRef} id={listId} role="listbox" aria-label="Files" className="shui-quick-open-list" data-stale={stale ? 'true' : undefined}>
          {listing && rows.length > 0 ? <div className="shui-quick-open-group">Recently opened</div> : null}
          {rows.map((row, index) => {
            const selected = index === active
            return (
              <button
                key={row.rel}
                type="button"
                id={`${listId}-${index}`}
                role="option"
                aria-selected={selected}
                data-index={index}
                className="shui-quick-open-row"
                title={row.rel}
                tabIndex={-1}
                onMouseMove={() => {
                  if (!selected) setActive(index)
                }}
                onClick={() => choose(row)}
              >
                <FileTypeIcon path={row.rel} className="file-icon" />
                <span className="name">
                  <Highlighted text={row.name} indices={row.indices} offset={row.nameOffset} />
                </span>
                {row.dir !== '' ? (
                  <span className="dir">
                    <span className="dir-text">
                      <Highlighted text={row.dir} indices={row.indices} offset={0} />
                    </span>
                  </span>
                ) : null}
              </button>
            )
          })}
          {note}
        </div>
        <div className="shui-quick-open-foot">
          <span className="hints">
            <kbd className="shui-launcher-key">↑</kbd>
            <kbd className="shui-launcher-key">↓</kbd> move
            <kbd className="shui-launcher-key">↵</kbd> open
            <kbd className="shui-launcher-key">esc</kbd> close
          </span>
          <span className="summary" role="status" aria-live="polite">
            {summary}
          </span>
        </div>
      </DialogContent>
    </Dialog>
  )
}

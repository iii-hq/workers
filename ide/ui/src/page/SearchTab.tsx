/* The Search view — VS Code's layout over the worker's `coder::search`:
   a query box with the Aa / ab / .* toggles inside it, include/exclude
   globs behind a disclosure, results grouped by file with the matched
   text highlighted inside a short window of its line, keyboard-walkable
   and virtualized so a thousand hits stay light. Searches run as you
   type (debounced), a stale response never overwrites a newer one. The
   Ask toggle sends the query to `coder::find-relevant` instead, on Enter
   only: the judge ranks files and the rows are its excerpts and leads. */

import { EmptyState, IconButton, SearchField } from '@iii-dev/console-ui'
import {
  CaseSensitive,
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  Ellipsis,
  Folder,
  Regex,
  RefreshCw,
  Sparkles,
  WholeWord,
  X,
} from 'lucide-react'
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '@iii-dev/console-ui/format'
import type { Host } from '@iii-dev/console-ui'
import { coderFindRelevant, coderSearch } from './coder'
import { FileTypeIcon } from './file-type-icon'
import {
  effectivePattern,
  flattenSearchRows,
  groupContentMatches,
  pathRows,
  relevantAsMatches,
  type SearchFileGroup,
  type SearchPathRow,
  type SearchRow,
  searchSummary,
  stepSearchRow,
} from './search-model'
import { ViewHeader } from './ViewHeader'
import { VirtualList } from './VirtualList'

const DEBOUNCE_MS = 220
const ROW_HEIGHT = 22
const MIN_AUTO_QUERY = 2
/** The worker's deadline for one ask; it answers `incomplete` when it runs out. */
const ASK_TIMEOUT_MS = 120_000
/** Nothing to highlight: the judge's rows carry no literal hit. */
const NO_HIGHLIGHT = { query: '', regex: false, ignoreCase: true, wholeWord: false }

/** An outside request to search: "Find in folder…" from the explorer. */
export interface SearchRequest {
  seq: number
  query?: string
  includeGlob?: string
}

interface SearchTabProps {
  host: Host
  root: string
  request: SearchRequest | null
  /** Click on a text match — open at the line as the preview tab. */
  onOpenMatch: (relPath: string, line: number, column: number, pin: boolean) => void
  /** Single click — open as the preview tab. */
  onPreviewFile: (relPath: string) => void
  /** Double click — open pinned. */
  onPinFile: (relPath: string) => void
  /** Click on a FOLDER match — expand + scroll to it in the explorer. */
  onRevealFolder: (relPath: string) => void
}

interface SearchResults {
  groups: SearchFileGroup[]
  paths: SearchPathRow[]
  truncated: boolean
}

/** A glob the user typed matches anywhere below the root: a bare pattern
    or a folder pattern both get the leading `**` segment VS Code implies. */
export function normalizeGlob(glob: string): string {
  const trimmed = glob.trim()
  if (trimmed === '') return ''
  if (trimmed.startsWith('**/') || trimmed.startsWith('/')) return trimmed.replace(/^\//, '')
  return `**/${trimmed}`
}

export function splitGlobs(text: string): string[] {
  return text
    .split(',')
    .map(normalizeGlob)
    .filter((glob) => glob !== '')
}

/** Module-level, so the memoized list sees the same function every render. */
const rowKey = (row: SearchRow) => row.key

function SearchTabView({ host, root, request, onOpenMatch, onPreviewFile, onPinFile, onRevealFolder }: SearchTabProps) {
  const [query, setQuery] = useState('')
  const [matchCase, setMatchCase] = useState(false)
  const [wholeWord, setWholeWord] = useState(false)
  const [regex, setRegex] = useState(false)
  const [ask, setAsk] = useState(false)
  const [askStartedAt, setAskStartedAt] = useState<number | null>(null)
  const [elapsed, setElapsed] = useState(0)
  const [detailsOpen, setDetailsOpen] = useState(false)
  const [includeGlob, setIncludeGlob] = useState('')
  const [excludeGlob, setExcludeGlob] = useState('')
  const [useGitignore, setUseGitignore] = useState(true)
  const [searching, setSearching] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [results, setResults] = useState<SearchResults | null>(null)
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set())
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(new Set())
  const [focusIndex, setFocusIndex] = useState(-1)
  const inputRef = useRef<HTMLInputElement>(null)
  // The page's search verb and the console's pane focus both look the box
  // up by attribute; the shared field carries no data-* props.
  useEffect(() => {
    inputRef.current?.setAttribute('data-shell-search-input', '')
    inputRef.current?.setAttribute('data-autofocus', '')
  }, [])
  const listRef = useRef<HTMLDivElement>(null)
  // Only the newest in-flight search may land — a slow older response
  // must not overwrite a newer one.
  const seqRef = useRef(0)
  const appliedRequestRef = useRef(0)

  const run = useCallback(
    (params: {
      query: string
      matchCase: boolean
      wholeWord: boolean
      regex: boolean
      includeGlob: string
      excludeGlob: string
      useGitignore: boolean
      ask: boolean
    }) => {
      const q = params.query
      if (q.trim() === '') {
        seqRef.current += 1
        setResults(null)
        setError(null)
        setSearching(false)
        setAskStartedAt(null)
        return
      }
      const seq = ++seqRef.current
      setSearching(true)
      setError(null)
      if (params.ask) {
        setAskStartedAt(Date.now())
        coderFindRelevant(host, { query: q, path: root, timeoutMs: ASK_TIMEOUT_MS })
          .then((out) => {
            if (seqRef.current !== seq) return
            if (out.status === 'unavailable') {
              setResults(null)
              setError(`Judge unavailable: ${out.reason ?? 'no reason given'} — use text search.`)
              return
            }
            setResults({
              groups: groupContentMatches(relevantAsMatches(out), root, NO_HIGHLIGHT),
              paths: [],
              truncated: out.status === 'incomplete',
            })
            setDismissed(new Set())
            setFocusIndex(-1)
          })
          .catch((err: unknown) => {
            if (seqRef.current !== seq) return
            setResults(null)
            setError(errorMessage(err))
          })
          .finally(() => {
            if (seqRef.current !== seq) return
            setSearching(false)
            setAskStartedAt(null)
          })
        return
      }
      setAskStartedAt(null)
      const options = { query: q, regex: params.regex, ignoreCase: !params.matchCase, wholeWord: params.wholeWord }
      const { pattern, regex: sendRegex } = effectivePattern(options)
      coderSearch(host, {
        query: pattern,
        regex: sendRegex,
        ignoreCase: !params.matchCase,
        path: root,
        includeGlobs: splitGlobs(params.includeGlob),
        excludeGlobs: splitGlobs(params.excludeGlob),
        respectGitignore: params.useGitignore,
        searchPaths: true,
      })
        .then((out) => {
          if (seqRef.current !== seq) return
          setResults({
            groups: groupContentMatches(out.content_matches, root, options),
            paths: pathRows(out, root),
            truncated: out.truncated,
          })
          setDismissed(new Set())
          setFocusIndex(-1)
        })
        .catch((err: unknown) => {
          if (seqRef.current !== seq) return
          setResults(null)
          setError(errorMessage(err))
        })
        .finally(() => {
          if (seqRef.current === seq) setSearching(false)
        })
    },
    [host, root],
  )

  // A new root orphans the answer and any search in flight: rows carry
  // paths relative to the old root, and a click would open them in the
  // new one. Text search re-runs below; an ask waits for Enter.
  const shownRootRef = useRef(root)
  useEffect(() => {
    if (shownRootRef.current === root) return
    shownRootRef.current = root
    seqRef.current += 1
    setResults(null)
    setError(null)
    setSearching(false)
    setAskStartedAt(null)
  }, [root])

  // Search as you type. An ask costs judge calls, so it runs on Enter only.
  useEffect(() => {
    if (query.trim().length < MIN_AUTO_QUERY) {
      if (query.trim() === '')
        run({ query: '', matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore, ask })
      return
    }
    if (ask) return
    const timer = window.setTimeout(
      () => run({ query, matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore, ask }),
      DEBOUNCE_MS,
    )
    return () => window.clearTimeout(timer)
  }, [query, matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore, ask, run])

  // Seconds since the running ask started; asks take tens of seconds.
  useEffect(() => {
    setElapsed(0)
    if (askStartedAt === null) return
    const timer = window.setInterval(() => setElapsed(Math.floor((Date.now() - askStartedAt) / 1000)), 1000)
    return () => window.clearInterval(timer)
  }, [askStartedAt])
  // The live region says only start and finish; the ticking seconds sit
  // beside it, hidden from screen readers.
  const searchingLabel = askStartedAt !== null ? 'asking the judge…' : 'searching…'

  useEffect(() => {
    if (!request || request.seq === appliedRequestRef.current) return
    appliedRequestRef.current = request.seq
    if (request.includeGlob !== undefined) {
      setIncludeGlob(request.includeGlob)
      setDetailsOpen(true)
    }
    if (request.query !== undefined) setQuery(request.query)
    window.requestAnimationFrame(() => {
      inputRef.current?.focus()
      inputRef.current?.select()
    })
  }, [request])

  const visibleGroups = useMemo(
    () => (results ? results.groups.filter((group) => !dismissed.has(group.path)) : []),
    [results, dismissed],
  )
  const rows = useMemo<SearchRow[]>(
    () => (results ? flattenSearchRows(visibleGroups, results.paths, collapsed) : []),
    [results, visibleGroups, collapsed],
  )
  // An incomplete ask has its own notice; "refine the query" is text-search advice.
  const summary = results ? searchSummary(visibleGroups, results.paths, results.truncated && !ask) : null
  const allCollapsed = visibleGroups.length > 0 && visibleGroups.every((group) => collapsed.has(group.path))

  const toggleGroup = useCallback((path: string) => {
    setCollapsed((previous) => {
      const next = new Set(previous)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })
  }, [])

  const activateRow = useCallback(
    (row: SearchRow, pin: boolean) => {
      if (row.type === 'match') onOpenMatch(row.group.rel, row.match.line, row.match.column, pin)
      else if (row.type === 'file') {
        if (pin) onPinFile(row.group.rel)
        else toggleGroup(row.group.path)
      } else if (row.type === 'path') {
        if (row.entry.kind === 'dir') onRevealFolder(row.entry.rel)
        else if (pin) onPinFile(row.entry.rel)
        else onPreviewFile(row.entry.rel)
      }
    },
    [onOpenMatch, onPinFile, onPreviewFile, onRevealFolder, toggleGroup],
  )

  const onListKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (rows.length === 0) return
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault()
        setFocusIndex((current) => {
          const start = current === -1 ? (event.key === 'ArrowDown' ? -1 : rows.length) : current
          return stepSearchRow(rows, start, event.key === 'ArrowDown' ? 1 : -1)
        })
        return
      }
      const row = rows[focusIndex]
      if (!row) return
      if (event.key === 'Enter') {
        event.preventDefault()
        activateRow(row, event.metaKey || event.ctrlKey)
      } else if (event.key === 'ArrowLeft' && row.type === 'file' && !row.collapsed) {
        event.preventDefault()
        toggleGroup(row.group.path)
      } else if (event.key === 'ArrowLeft' && row.type === 'match') {
        event.preventDefault()
        const parent = rows.findIndex((r) => r.type === 'file' && r.group.path === row.group.path)
        if (parent !== -1) setFocusIndex(parent)
      } else if (event.key === 'ArrowRight' && row.type === 'file' && row.collapsed) {
        event.preventDefault()
        toggleGroup(row.group.path)
      } else if (event.key === 'Escape') {
        event.preventDefault()
        inputRef.current?.focus()
      }
    },
    [rows, focusIndex, activateRow, toggleGroup],
  )

  const renderRow = useCallback(
    (row: SearchRow, index: number) => {
      const focused = index === focusIndex
      if (row.type === 'section') {
        return (
          <div className="shui-search-section">
            <span>{row.label}</span>
            <span className="count">{row.count}</span>
          </div>
        )
      }
      if (row.type === 'file') {
        return (
          <div
            className={`shui-search-file${focused ? ' focused' : ''}`}
            role="treeitem"
            tabIndex={-1}
            aria-expanded={!row.collapsed}
            aria-selected={focused}
            title={row.group.rel}
            onClick={() => {
              setFocusIndex(index)
              toggleGroup(row.group.path)
            }}
            onDoubleClick={() => onPinFile(row.group.rel)}
            onKeyDown={undefined}
          >
            {row.collapsed ? <ChevronRight aria-hidden className="chevron" /> : <ChevronDown aria-hidden className="chevron" />}
            <FileTypeIcon path={row.group.rel} className="file-icon" />
            <span className="name">{row.group.name}</span>
            {row.group.dir ? <span className="dir">{row.group.dir}</span> : null}
            <span className="spacer" />
            <span className="count">{row.group.matches.length}</span>
            <button
              type="button"
              className="dismiss"
              aria-label={`Dismiss results in ${row.group.name}`}
              onClick={(event) => {
                event.stopPropagation()
                setDismissed((previous) => new Set(previous).add(row.group.path))
              }}
            >
              <X aria-hidden />
            </button>
          </div>
        )
      }
      if (row.type === 'path') {
        return (
          <div
            className={`shui-search-path${focused ? ' focused' : ''}`}
            role="treeitem"
            tabIndex={-1}
            aria-selected={focused}
            title={row.entry.rel}
            onClick={() => {
              setFocusIndex(index)
              activateRow(row, false)
            }}
            onDoubleClick={() => activateRow(row, true)}
            onKeyDown={undefined}
          >
            {row.entry.kind === 'dir' ? (
              <Folder aria-hidden className="file-icon folder" />
            ) : (
              <FileTypeIcon path={row.entry.rel} className="file-icon" />
            )}
            <span className="name">{row.entry.name}</span>
            {row.entry.dir ? <span className="dir">{row.entry.dir}</span> : null}
          </div>
        )
      }
      const { match } = row
      return (
        <div
          className={`shui-search-match${focused ? ' focused' : ''}`}
          role="treeitem"
          tabIndex={-1}
          aria-selected={focused}
          title={`${row.group.rel}:${match.line}:${match.column}`}
          onClick={() => {
            setFocusIndex(index)
            activateRow(row, false)
          }}
          onDoubleClick={() => activateRow(row, true)}
          onKeyDown={undefined}
        >
          <span className="line">{match.line}</span>
          <span className="text">
            {match.leadCut ? <span className="cut">…</span> : null}
            {match.lead}
            {match.hit ? <mark className="shui-hl">{match.hit}</mark> : null}
            {match.trail}
          </span>
        </div>
      )
    },
    [focusIndex, toggleGroup, onPinFile, activateRow],
  )

  return (
    <div className="shui-search">
      <ViewHeader
        title="Search"
        actions={
          <>
            <IconButton
              label="Refresh"
              disabled={query.trim() === ''}
              onClick={() => run({ query, matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore, ask })}
            >
              <RefreshCw aria-hidden />
            </IconButton>
            <IconButton
              label="Clear search results"
              disabled={query === '' && results === null}
              onClick={() => {
                setQuery('')
                setResults(null)
                setError(null)
                inputRef.current?.focus()
              }}
            >
              <X aria-hidden />
            </IconButton>
            <IconButton
              label={allCollapsed ? 'Expand all' : 'Collapse all'}
              disabled={visibleGroups.length === 0}
              onClick={() =>
                setCollapsed(allCollapsed ? new Set() : new Set(visibleGroups.map((group) => group.path)))
              }
            >
              {allCollapsed ? <ChevronsUpDown aria-hidden /> : <ChevronsDownUp aria-hidden />}
            </IconButton>
          </>
        }
      />
      <form
        className="shui-search-form"
        onSubmit={(event) => {
          event.preventDefault()
          run({ query, matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore, ask })
        }}
      >
        <div className="shui-search-box">
          <SearchField
            ref={inputRef}
            value={query}
            onChange={setQuery}
            placeholder={ask ? 'Ask what the code does, then Enter' : 'Search'}
            aria-label="Search query"
            className="shui-search-query"
            onKeyDown={(event) => {
              // Run here, not by implicit form submission: with the details
              // open the form has three text fields and no submit button,
              // so the browser's Enter-to-submit never fires.
              if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                event.preventDefault()
                run({ query, matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore, ask })
              } else if (event.key === 'ArrowDown' && rows.length > 0) {
                event.preventDefault()
                setFocusIndex(stepSearchRow(rows, -1, 1))
                listRef.current?.focus()
              }
            }}
          />
          <span className="shui-search-toggles">
            {ask ? null : (
              <>
                <SearchToggle label="Match case" pressed={matchCase} onToggle={() => setMatchCase((value) => !value)}>
                  <CaseSensitive aria-hidden />
                </SearchToggle>
                <SearchToggle
                  label="Match whole word"
                  pressed={wholeWord}
                  onToggle={() => setWholeWord((value) => !value)}
                >
                  <WholeWord aria-hidden />
                </SearchToggle>
                <SearchToggle
                  label="Use regular expression"
                  pressed={regex}
                  onToggle={() => setRegex((value) => !value)}
                >
                  <Regex aria-hidden />
                </SearchToggle>
              </>
            )}
            <SearchToggle
              label="Ask the judge"
              title="Ask the judge which code does this (Enter to run; file text is sent to the judge provider)"
              pressed={ask}
              onToggle={() => {
                // Drop the other mode's answer; leaving ask re-runs text search.
                seqRef.current += 1
                setResults(null)
                setError(null)
                setSearching(false)
                setAskStartedAt(null)
                setAsk((value) => !value)
              }}
            >
              <Sparkles aria-hidden />
            </SearchToggle>
          </span>
        </div>
        <div className="shui-search-details-row">
          <button
            type="button"
            className={`shui-search-details-toggle${detailsOpen ? ' open' : ''}`}
            aria-expanded={detailsOpen}
            aria-label="Toggle search details"
            onClick={() => setDetailsOpen((value) => !value)}
          >
            <Ellipsis aria-hidden />
          </button>
          <span className="shui-search-summary">
            <span role="status">{searching ? searchingLabel : (summary ?? '')}</span>
            {askStartedAt !== null ? <span aria-hidden="true"> {elapsed}s</span> : null}
          </span>
        </div>
        {detailsOpen ? (
          <div className="shui-search-details">
            <label className="shui-search-field">
              <span>files to include</span>
              <input
                type="text"
                value={includeGlob}
                onChange={(event) => setIncludeGlob(event.target.value)}
                placeholder="e.g. *.ts, src/**"
                autoComplete="off"
                spellCheck={false}
              />
            </label>
            <label className="shui-search-field">
              <span>files to exclude</span>
              <input
                type="text"
                value={excludeGlob}
                onChange={(event) => setExcludeGlob(event.target.value)}
                placeholder="e.g. *.test.ts, dist/**"
                autoComplete="off"
                spellCheck={false}
              />
            </label>
            <label className="shui-search-check">
              <input type="checkbox" checked={useGitignore} onChange={(event) => setUseGitignore(event.target.checked)} />
              <span>skip files ignored by Git</span>
            </label>
          </div>
        ) : null}
      </form>

      {error ? <div className="shui-side-note warn">{error}</div> : null}
      {results && rows.length === 0 && !searching ? (
        <div className="shui-side-empty">
          <EmptyState title="No results" description="Nothing matched. Review the query and the configured exclusions." />
        </div>
      ) : null}
      {results?.truncated ? (
        <div className="shui-search-truncated">
          {ask
            ? 'The judge did not finish — partial results; narrow the question or the folder.'
            : 'Showing the first results only — narrow the query or the folder.'}
        </div>
      ) : null}
      {rows.length > 0 ? (
        <VirtualList
          rows={rows}
          rowHeight={ROW_HEIGHT}
          renderRow={renderRow}
          rowKey={rowKey}
          className="shui-search-results"
          scrollToIndex={focusIndex}
          role="tree"
          aria-label="Search results"
          tabIndex={0}
          onKeyDown={onListKeyDown}
          listRef={listRef}
        />
      ) : null}
    </div>
  )
}

/** Memoized: the page re-renders often, and this only when its props change. */
export const SearchTab = memo(SearchTabView)

function SearchToggle({
  label,
  title = label,
  pressed,
  onToggle,
  children,
}: {
  label: string
  title?: string
  pressed: boolean
  onToggle: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      className={`shui-search-toggle${pressed ? ' active' : ''}`}
      aria-label={label}
      aria-pressed={pressed}
      title={title}
      onClick={onToggle}
    >
      {children}
    </button>
  )
}

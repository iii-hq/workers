/* The Search view — VS Code's layout over the worker's `coder::search`:
   a query box with the Aa / ab / .* toggles inside it, include/exclude
   globs behind a disclosure, results grouped by file with the matched
   text highlighted inside a short window of its line, keyboard-walkable
   and virtualized so a thousand hits stay light. Searches run as you
   type (debounced), a stale response never overwrites a newer one. */

import { IconButton } from '@iii-dev/console-ui'
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
  WholeWord,
  X,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import type { Host } from '@iii-dev/console-ui'
import { coderSearch } from './coder'
import { FileTypeIcon } from './file-type-icon'
import {
  effectivePattern,
  flattenSearchRows,
  groupContentMatches,
  pathRows,
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

export function SearchTab({
  host,
  root,
  request,
  onOpenMatch,
  onPreviewFile,
  onPinFile,
  onRevealFolder,
}: SearchTabProps) {
  const [query, setQuery] = useState('')
  const [matchCase, setMatchCase] = useState(false)
  const [wholeWord, setWholeWord] = useState(false)
  const [regex, setRegex] = useState(false)
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
    }) => {
      const q = params.query
      if (q.trim() === '') {
        seqRef.current += 1
        setResults(null)
        setError(null)
        setSearching(false)
        return
      }
      const seq = ++seqRef.current
      setSearching(true)
      setError(null)
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

  // Search as you type. A fresh root clears the old answer.
  useEffect(() => {
    if (query.trim().length < MIN_AUTO_QUERY) {
      if (query.trim() === '') run({ query: '', matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore })
      return
    }
    const timer = window.setTimeout(
      () => run({ query, matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore }),
      DEBOUNCE_MS,
    )
    return () => window.clearTimeout(timer)
  }, [query, matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore, run])

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
  const summary = results ? searchSummary(visibleGroups, results.paths, results.truncated) : null
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
              onClick={() => run({ query, matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore })}
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
          run({ query, matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore })
        }}
      >
        <div className="shui-search-box">
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search"
            aria-label="Search query"
            autoComplete="off"
            spellCheck={false}
            data-shell-search-input=""
            data-autofocus=""
            onKeyDown={(event) => {
              if (event.key === 'ArrowDown' && rows.length > 0) {
                event.preventDefault()
                setFocusIndex(stepSearchRow(rows, -1, 1))
                listRef.current?.focus()
              } else if (event.key === 'Escape' && query !== '') {
                event.preventDefault()
                setQuery('')
              }
            }}
          />
          <span className="shui-search-toggles">
            <SearchToggle label="Match case" pressed={matchCase} onToggle={() => setMatchCase((value) => !value)}>
              <CaseSensitive aria-hidden />
            </SearchToggle>
            <SearchToggle label="Match whole word" pressed={wholeWord} onToggle={() => setWholeWord((value) => !value)}>
              <WholeWord aria-hidden />
            </SearchToggle>
            <SearchToggle label="Use regular expression" pressed={regex} onToggle={() => setRegex((value) => !value)}>
              <Regex aria-hidden />
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
          {summary !== null ? (
            <span className="shui-search-summary" role="status">
              {searching ? 'searching…' : summary}
            </span>
          ) : searching ? (
            <span className="shui-search-summary" role="status">
              searching…
            </span>
          ) : null}
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
        <div className="shui-side-note">No results found. Review your settings for configured exclusions.</div>
      ) : null}
      {results?.truncated ? (
        <div className="shui-search-truncated">Showing the first results only — narrow the query or the folder.</div>
      ) : null}
      {rows.length > 0 ? (
        <VirtualList
          rows={rows}
          rowHeight={ROW_HEIGHT}
          renderRow={renderRow}
          rowKey={(row) => row.key}
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

function SearchToggle({
  label,
  pressed,
  onToggle,
  children,
}: {
  label: string
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
      title={label}
      onClick={onToggle}
    >
      {children}
    </button>
  )
}

/* The Search view — VS Code's layout over the worker's `coder::search`:
   a query box with the Aa / ab / .* toggles inside it and a Replace box
   under it behind the left chevron, include/exclude globs behind a
   disclosure, results grouped by file with the matched text highlighted
   inside a short window of its line (and the replacement previewed over
   it), keyboard-walkable and virtualized so a thousand hits stay light.
   Searches run as you type (debounced), a stale response never overwrites
   a newer one. */

import { EmptyState, IconButton } from '@iii-dev/console-ui'
import {
  BookOpen,
  CaseSensitive,
  CaseUpper,
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  Folder,
  Regex,
  RefreshCw,
  Replace,
  ReplaceAll,
  Settings,
  WholeWord,
  X,
} from 'lucide-react'
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '@iii-dev/console-ui/format'
import type { ConfirmOptions, Host } from '@iii-dev/console-ui'
import { coderSearch, joinPath } from './coder'
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
import {
  type FileReplaceOutcome,
  previewReplacement,
  replaceAllInText,
  replaceInFile,
  replaceOneInText,
  type ReplaceOptions,
  replaceSummary,
} from './search-replace'
import { ViewHeader } from './ViewHeader'
import { createPaneMemory } from './pane-memory'
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
  /** The pane the view belongs to: what was typed comes back per pane. */
  memoryKey: string
  request: SearchRequest | null
  /** Click on a text match — open at the line as the preview tab. */
  onOpenMatch: (relPath: string, line: number, column: number, pin: boolean) => void
  /** Single click — open as the preview tab. */
  onPreviewFile: (relPath: string) => void
  /** Double click — open pinned. */
  onPinFile: (relPath: string) => void
  /** Click on a FOLDER match — expand + scroll to it in the explorer. */
  onRevealFolder: (relPath: string) => void
  /** Root-relative paths with unsaved edits: a replace leaves them alone. */
  dirtyPaths: ReadonlySet<string>
  /** Root-relative paths open in editor tabs, for "search only in open editors". */
  openPaths: readonly string[]
  /** The page's confirmation, asked before a replace across files. */
  confirm: (options: ConfirmOptions) => Promise<boolean>
}

interface SearchResults {
  groups: SearchFileGroup[]
  paths: SearchPathRow[]
  truncated: boolean
}

/** What the view held when it last unmounted (another sidebar view, or the
    page remounting), per pane: the form as typed and set, the request it
    already applied, and the last answer with the folder it came from. In
    memory only (`pane-memory.ts`): a query or replacement can hold secrets. */
interface SearchMemory {
  query: string
  matchCase: boolean
  wholeWord: boolean
  regex: boolean
  includeGlob: string
  excludeGlob: string
  useGitignore: boolean
  onlyOpenEditors: boolean
  replacement: string
  preserveCase: boolean
  /** By identity: a request is the page's object, so one made after the page
      remounted is new even if its seq repeats an old one. */
  appliedRequest: SearchRequest | null
  answer: { root: string; results: SearchResults; collapsed: ReadonlySet<string>; dismissed: ReadonlySet<string> } | null
}

const searchMemory = createPaneMemory<SearchMemory>()

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

interface SearchParams {
  query: string
  matchCase: boolean
  wholeWord: boolean
  regex: boolean
  includeGlob: string
  excludeGlob: string
  useGitignore: boolean
  /** Root-relative paths to search instead of the include globs; null = all. */
  scopePaths: readonly string[] | null
}

/** Module-level, so the memoized list sees the same function every render. */
const rowKey = (row: SearchRow) => row.key

/** Files rewritten at once by a replace across files. */
const REPLACE_CONCURRENCY = 4

function SearchTabView({
  host,
  root,
  memoryKey,
  request,
  onOpenMatch,
  onPreviewFile,
  onPinFile,
  onRevealFolder,
  dirtyPaths,
  openPaths,
  confirm,
}: SearchTabProps) {
  const [remembered] = useState(() => searchMemory.read(memoryKey))
  // The last answer shows at once if it is this folder's; the search runs
  // again behind it, since the files may have moved on.
  const [answer] = useState(() => (remembered?.answer?.root === root ? remembered.answer : null))
  const [query, setQuery] = useState(remembered?.query ?? '')
  const [matchCase, setMatchCase] = useState(remembered?.matchCase ?? false)
  const [wholeWord, setWholeWord] = useState(remembered?.wholeWord ?? false)
  const [regex, setRegex] = useState(remembered?.regex ?? false)
  const [includeGlob, setIncludeGlob] = useState(remembered?.includeGlob ?? '')
  const [excludeGlob, setExcludeGlob] = useState(remembered?.excludeGlob ?? '')
  const [useGitignore, setUseGitignore] = useState(remembered?.useGitignore ?? true)
  const [onlyOpenEditors, setOnlyOpenEditors] = useState(remembered?.onlyOpenEditors ?? false)
  const [replacement, setReplacement] = useState(remembered?.replacement ?? '')
  const [preserveCase, setPreserveCase] = useState(remembered?.preserveCase ?? false)
  const [replacing, setReplacing] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)
  const [searching, setSearching] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [results, setResults] = useState<SearchResults | null>(answer?.results ?? null)
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(answer?.collapsed ?? new Set())
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(answer?.dismissed ?? new Set())
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
  const appliedRequestRef = useRef<SearchRequest | null>(remembered?.appliedRequest ?? null)

  // Open editors only: the open tabs' own paths are the include list.
  const scopePaths = onlyOpenEditors ? openPaths : null
  const params = useMemo<SearchParams>(
    () => ({ query, matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore, scopePaths }),
    [query, matchCase, wholeWord, regex, includeGlob, excludeGlob, useGitignore, scopePaths],
  )

  const run = useCallback(
    (params: SearchParams) => {
      const q = params.query
      if (q.trim() === '' || params.scopePaths?.length === 0) {
        seqRef.current += 1
        setResults(q.trim() === '' ? null : { groups: [], paths: [], truncated: false })
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
        includeGlobs: params.scopePaths ?? splitGlobs(params.includeGlob),
        excludeGlobs: splitGlobs(params.excludeGlob),
        respectGitignore: params.useGitignore,
        searchPaths: params.scopePaths === null,
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
    if (params.query.trim().length < MIN_AUTO_QUERY) {
      if (params.query.trim() === '') run(params)
      return
    }
    const timer = window.setTimeout(() => run(params), DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [params, run])

  useEffect(() => {
    if (!request || request === appliedRequestRef.current) return
    appliedRequestRef.current = request
    if (request.includeGlob !== undefined) setIncludeGlob(request.includeGlob)
    if (request.query !== undefined) setQuery(request.query)
    window.requestAnimationFrame(() => {
      inputRef.current?.focus()
      inputRef.current?.select()
    })
  }, [request])

  // Kept as it changes (a map write, no render), so whatever unmounts the
  // view finds it current.
  useEffect(() => {
    searchMemory.write(memoryKey, {
      query,
      matchCase,
      wholeWord,
      regex,
      includeGlob,
      excludeGlob,
      useGitignore,
      onlyOpenEditors,
      replacement,
      preserveCase,
      appliedRequest: appliedRequestRef.current,
      answer: results ? { root, results, collapsed, dismissed } : null,
    })
  }, [
    memoryKey,
    root,
    query,
    matchCase,
    wholeWord,
    regex,
    includeGlob,
    excludeGlob,
    useGitignore,
    onlyOpenEditors,
    replacement,
    preserveCase,
    results,
    collapsed,
    dismissed,
    request,
  ])

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

  // A replace's outcome stands until the query or the replacement moves on.
  useEffect(() => setNotice(null), [query, replacement])

  // ── replace ──
  // The preview and the rewrite use the pattern the results came from.
  const replaceOptions = useMemo<ReplaceOptions>(
    () => ({ query, regex, ignoreCase: !matchCase, wholeWord, replacement, preserveCase }),
    [query, regex, matchCase, wholeWord, replacement, preserveCase],
  )
  const showReplace = query.trim() !== ''
  const previewing = showReplace && replacement !== ''

  /** Rewrite `groups` with `edit`, skipping files with unsaved edits, then
      search again so the results show what is left. */
  const replaceAcross = useCallback(
    async (groups: readonly SearchFileGroup[], edit: (content: string) => { content: string; count: number }) => {
      if (groups.length === 0) return
      setReplacing(true)
      setNotice(null)
      const outcomes: FileReplaceOutcome[] = []
      const queue = [...groups]
      const worker = async () => {
        for (let group = queue.shift(); group; group = queue.shift()) {
          const abs = joinPath(root, group.rel)
          if (dirtyPaths.has(group.rel)) {
            outcomes.push({ path: abs, status: 'skipped', reason: `${group.name} has unsaved edits` })
            continue
          }
          try {
            outcomes.push(await replaceInFile(host, abs, edit))
          } catch (err: unknown) {
            outcomes.push({ path: abs, status: 'skipped', reason: errorMessage(err) })
          }
        }
      }
      await Promise.all(Array.from({ length: Math.min(REPLACE_CONCURRENCY, queue.length) }, worker))
      setReplacing(false)
      setNotice(replaceSummary(outcomes))
      run(params)
    },
    [host, root, dirtyPaths, run, params],
  )

  const replaceAll = useCallback(async () => {
    const total = visibleGroups.reduce((sum, group) => sum + group.matches.length, 0)
    if (total === 0 || replacing) return
    const files = visibleGroups.length
    const ok = await confirm({
      title: `Replace ${total} ${total === 1 ? 'occurrence' : 'occurrences'} across ${files} ${files === 1 ? 'file' : 'files'}?`,
      description:
        replacement === ''
          ? `Every match of “${query}” is deleted. Files with unsaved edits are skipped.`
          : `Every match of “${query}” becomes “${replacement}”. Files with unsaved edits are skipped.`,
      confirmLabel: 'Replace all',
    })
    if (ok) await replaceAcross(visibleGroups, (content) => replaceAllInText(content, replaceOptions))
  }, [visibleGroups, replacing, confirm, replacement, query, replaceAcross, replaceOptions])

  const replaceFile = useCallback(
    (group: SearchFileGroup) => {
      if (!replacing) void replaceAcross([group], (content) => replaceAllInText(content, replaceOptions))
    },
    [replacing, replaceAcross, replaceOptions],
  )

  const replaceMatch = useCallback(
    (group: SearchFileGroup, line: number, column: number) => {
      if (!replacing) void replaceAcross([group], (content) => replaceOneInText(content, line, column, replaceOptions))
    },
    [replacing, replaceAcross, replaceOptions],
  )

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
            <span className="shui-search-row-actions">
              {showReplace ? (
                <button
                  type="button"
                  aria-label={`Replace all in ${row.group.name}`}
                  title="Replace all in this file"
                  disabled={replacing}
                  onClick={(event) => {
                    event.stopPropagation()
                    replaceFile(row.group)
                  }}
                >
                  <ReplaceAll aria-hidden />
                </button>
              ) : null}
              <button
                type="button"
                aria-label={`Dismiss results in ${row.group.name}`}
                title="Dismiss"
                onClick={(event) => {
                  event.stopPropagation()
                  setDismissed((previous) => new Set(previous).add(row.group.path))
                }}
              >
                <X aria-hidden />
              </button>
            </span>
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
            {match.hit && previewing ? (
              <>
                <del className="shui-hl-del">{match.hit}</del>
                <ins className="shui-hl-ins">{previewReplacement(match.hit, replaceOptions)}</ins>
              </>
            ) : match.hit ? (
              <mark className="shui-hl">{match.hit}</mark>
            ) : null}
            {match.trail}
          </span>
          {showReplace && match.hit ? (
            <span className="shui-search-row-actions">
              <button
                type="button"
                aria-label={`Replace on line ${match.line}`}
                title="Replace"
                disabled={replacing}
                onClick={(event) => {
                  event.stopPropagation()
                  replaceMatch(row.group, match.line, match.column)
                }}
              >
                <Replace aria-hidden />
              </button>
            </span>
          ) : null}
        </div>
      )
    },
    [focusIndex, toggleGroup, onPinFile, activateRow, showReplace, previewing, replacing, replaceOptions, replaceFile, replaceMatch],
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
              onClick={() => run(params)}
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
          run(params)
        }}
      >
        {/* Four fields, always there (VS Code's order): search, replace, the
            files to include, the files to exclude. Nothing to open first. */}
        <div className="shui-search-fields">
          <div className="shui-search-input">
            <input
              ref={inputRef}
              type="text"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search"
              aria-label="Search query"
              autoComplete="off"
              spellCheck={false}
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
          <div className="shui-search-replace-row">
            <div className="shui-search-input">
              <input
                type="text"
                value={replacement}
                onChange={(event) => setReplacement(event.target.value)}
                placeholder="Replace"
                aria-label="Replace"
                autoComplete="off"
                spellCheck={false}
                onKeyDown={(event) => {
                  // ⌘⏎ replaces everywhere, the way VS Code's box does.
                  if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
                    event.preventDefault()
                    void replaceAll()
                  }
                }}
              />
              <span className="shui-search-toggles">
                <SearchToggle label="Preserve case" pressed={preserveCase} onToggle={() => setPreserveCase((value) => !value)}>
                  <CaseUpper aria-hidden />
                </SearchToggle>
              </span>
            </div>
            <button
              type="button"
              className="shui-search-toggle shui-search-replace-all"
              aria-label="Replace all"
              title="Replace all (⌘⏎)"
              disabled={!showReplace || visibleGroups.length === 0 || replacing}
              onClick={() => void replaceAll()}
            >
              <ReplaceAll aria-hidden />
            </button>
          </div>
        </div>
        <div className="shui-search-details">
          <label className="shui-search-field">
            <span>Files to include</span>
            <span className="shui-search-input">
              <input
                type="text"
                value={includeGlob}
                onChange={(event) => setIncludeGlob(event.target.value)}
                placeholder={onlyOpenEditors ? 'open editors' : 'e.g. *.ts, src/**'}
                disabled={onlyOpenEditors}
                autoComplete="off"
                spellCheck={false}
              />
              <span className="shui-search-toggles">
                <SearchToggle
                  label="Search only in open editors"
                  pressed={onlyOpenEditors}
                  onToggle={() => setOnlyOpenEditors((value) => !value)}
                >
                  <BookOpen aria-hidden />
                </SearchToggle>
              </span>
            </span>
          </label>
          <label className="shui-search-field">
            <span>Files to exclude</span>
            <span className="shui-search-input">
              <input
                type="text"
                value={excludeGlob}
                onChange={(event) => setExcludeGlob(event.target.value)}
                placeholder="e.g. *.test.ts, dist/**"
                autoComplete="off"
                spellCheck={false}
              />
              <span className="shui-search-toggles">
                <SearchToggle
                  label="Use exclude settings and ignore files"
                  pressed={useGitignore}
                  onToggle={() => setUseGitignore((value) => !value)}
                >
                  <Settings aria-hidden />
                </SearchToggle>
              </span>
            </span>
          </label>
        </div>
        {replacing || searching || notice || summary ? (
          <span className="shui-search-summary" role="status">
            {replacing ? 'replacing…' : searching ? 'searching…' : (notice ?? summary ?? '')}
          </span>
        ) : null}
      </form>

      {error ? <div className="shui-side-note warn">{error}</div> : null}
      {results && rows.length === 0 && !searching ? (
        <div className="shui-side-empty">
          <EmptyState title="No results" description="Nothing matched. Review the query and the configured exclusions." />
        </div>
      ) : null}
      {results?.truncated ? (
        <div className="shui-search-truncated">Showing the first results only — narrow the query or the folder.</div>
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

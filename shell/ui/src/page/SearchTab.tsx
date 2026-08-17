/* The search tab — content + path search over the browsed root via the
   worker's `coder::search`. Rows open the file in the editor pane. */

import { Input } from '@iii-dev/console-ui'
import { useCallback, useRef, useState } from 'react'
import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '../lib/format'
import { renderWithHighlight } from '../lib/highlight'
import { coderSearch, relativeTo, type SearchResponse } from './coder'

interface SearchTabProps {
  host: Host
  root: string
  /** Single click — open as the preview tab. */
  onPreviewFile: (relPath: string) => void
  /** Double click — open pinned. */
  onPinFile: (relPath: string) => void
  /** Click on a FOLDER match — expand + scroll to it in the files tab. */
  onRevealFolder: (relPath: string) => void
}

export function SearchTab({
  host,
  root,
  onPreviewFile,
  onPinFile,
  onRevealFolder,
}: SearchTabProps) {
  const [query, setQuery] = useState('')
  const [regex, setRegex] = useState(false)
  const [ignoreCase, setIgnoreCase] = useState(true)
  const [searching, setSearching] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<SearchResponse | null>(null)
  // Only the newest in-flight search may land — a slow older response
  // must not overwrite a newer one.
  const seqRef = useRef(0)

  const run = useCallback(() => {
    const q = query.trim()
    if (!q) return
    const seq = ++seqRef.current
    setSearching(true)
    setError(null)
    coderSearch(host, { query: q, regex, ignoreCase, path: root })
      .then((out) => {
        if (seqRef.current !== seq) return
        setResult(out)
      })
      .catch((err: unknown) => {
        if (seqRef.current !== seq) return
        setResult(null)
        setError(errorMessage(err))
      })
      .finally(() => {
        if (seqRef.current === seq) setSearching(false)
      })
  }, [host, query, regex, ignoreCase, root])

  return (
    <div className="shui-search">
      <form
        className="shui-search-form"
        onSubmit={(e) => {
          e.preventDefault()
          run()
        }}
      >
        <Input
          value={query}
          onChange={setQuery}
          placeholder="search files & folders…"
          preserveCase
          aria-label="search query"
        />
        <div className="shui-search-opts">
          <label className="opt">
            <input
              type="checkbox"
              checked={regex}
              onChange={(e) => setRegex(e.target.checked)}
            />
            regex
          </label>
          <label className="opt">
            <input
              type="checkbox"
              checked={ignoreCase}
              onChange={(e) => setIgnoreCase(e.target.checked)}
            />
            ignore case
          </label>
          <button type="submit" className="shui-linkish" disabled={searching}>
            {searching ? 'searching…' : 'search'}
          </button>
        </div>
      </form>

      {error ? <div className="shui-side-note warn">{error}</div> : null}

      {result ? (
        <SearchResults
          result={result}
          root={root}
          query={query}
          regex={regex}
          ignoreCase={ignoreCase}
          onPreviewFile={onPreviewFile}
          onPinFile={onPinFile}
          onRevealFolder={onRevealFolder}
        />
      ) : null}
    </div>
  )
}

function SearchResults({
  result,
  root,
  query,
  regex,
  ignoreCase,
  onPreviewFile,
  onPinFile,
  onRevealFolder,
}: {
  result: SearchResponse
  root: string
  query: string
  regex: boolean
  ignoreCase: boolean
  onPreviewFile: (relPath: string) => void
  onPinFile: (relPath: string) => void
  onRevealFolder: (relPath: string) => void
}) {
  const { content_matches, path_matches, truncated } = result
  if (content_matches.length === 0 && path_matches.length === 0) {
    return <div className="shui-side-note">· no matches</div>
  }
  const folderMatches = path_matches.filter((m) => m.kind === 'dir')
  const fileMatches = path_matches.filter((m) => m.kind !== 'dir')
  return (
    <div className="shui-search-results">
      {truncated ? (
        <div className="shui-side-note warn">
          truncated — refine the query
        </div>
      ) : null}

      {folderMatches.length > 0 ? (
        <>
          <div className="shui-side-note head">folders</div>
          {folderMatches.map((m) => {
            const rel = relativeTo(root, m.path)
            return (
              <button
                key={m.path}
                type="button"
                className="shui-search-row"
                onClick={() => onRevealFolder(rel)}
                title={`reveal ${rel} in files`}
              >
                <span className="loc">{rel}/</span>
              </button>
            )
          })}
        </>
      ) : null}

      {fileMatches.length > 0 ? (
        <>
          <div className="shui-side-note head">files</div>
          {fileMatches.map((m) => {
            const rel = relativeTo(root, m.path)
            return (
              <button
                key={m.path}
                type="button"
                className="shui-search-row"
                onClick={() => onPreviewFile(rel)}
                onDoubleClick={() => onPinFile(rel)}
                title={rel}
              >
                <span className="loc">{rel}</span>
              </button>
            )
          })}
        </>
      ) : null}

      {content_matches.length > 0 ? (
        <>
          <div className="shui-side-note head">matches</div>
          {content_matches.map((m) => {
            const rel = relativeTo(root, m.path)
            return (
              <button
                key={`${m.path}:${m.line}:${m.column}`}
                type="button"
                className="shui-search-row"
                onClick={() => onPreviewFile(rel)}
                onDoubleClick={() => onPinFile(rel)}
                title={`${rel}:${m.line}`}
              >
                <span className="loc">
                  {rel}
                  <span className="line">:{m.line}</span>
                </span>
                <span className="text">
                  {renderWithHighlight(m.text, query, {
                    isRegex: regex,
                    ignoreCase,
                  })}
                </span>
              </button>
            )
          })}
        </>
      ) : null}
    </div>
  )
}

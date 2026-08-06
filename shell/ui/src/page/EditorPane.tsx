/* The main editing surface — the console's shared Monaco-backed
   CodeEditor over `coder::read-file` / `coder::create-file(overwrite)`.
   Never bundles an editor: Monaco ships once, inside the console.

   File content + unsaved drafts live in a page-owned cache keyed by
   path, so switching editor tabs never discards edits: the pane is
   remounted per file (key=path) and rehydrates from the cache instead
   of re-reading. */

import { CodeEditor, type Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { errorMessage, formatBytes } from '../lib/format'
import { coderReadFile, coderWriteFile, joinPath } from './coder'

/** File-extension → Monaco language id (unknown ids render plain). */
export function monacoLangFromPath(path: string): string {
  const lower = path.toLowerCase()
  if (lower.endsWith('dockerfile')) return 'dockerfile'
  if (lower.endsWith('makefile')) return 'shell'
  const ext = lower.match(/\.([a-z0-9]+)$/)?.[1]
  switch (ext) {
    case 'ts':
    case 'tsx':
      return 'typescript'
    case 'js':
    case 'jsx':
    case 'mjs':
    case 'cjs':
      return 'javascript'
    case 'json':
    case 'jsonc':
      return 'json'
    case 'yml':
    case 'yaml':
      return 'yaml'
    case 'md':
    case 'mdx':
      return 'markdown'
    case 'rs':
      return 'rust'
    case 'go':
      return 'go'
    case 'py':
    case 'pyi':
      return 'python'
    case 'rb':
      return 'ruby'
    case 'sh':
    case 'bash':
    case 'zsh':
      return 'shell'
    case 'html':
    case 'htm':
      return 'html'
    case 'css':
      return 'css'
    case 'sql':
      return 'sql'
    case 'xml':
      return 'xml'
    case 'toml':
    case 'ini':
      return 'ini'
    default:
      return 'plaintext'
  }
}

/** Read-only reasons: binary content, or a byte-capped read (saving a
    truncated body would destroy the tail). */
export type ReadOnlyReason = 'binary' | 'truncated' | null

export interface EditorCacheEntry {
  /** What the worker last gave (or accepted) for this file. */
  savedContent: string
  /** The live buffer, possibly unsaved. */
  draft: string
  readOnly: ReadOnlyReason
  mode: number | null
  size: number | null
}

/** Page-owned; survives tab switches, dropped when a tab closes. */
export type EditorCache = Map<string, EditorCacheEntry>

type PaneState =
  | { phase: 'loading' }
  | { phase: 'error'; message: string }
  | { phase: 'ready' }

interface EditorPaneProps {
  host: Host
  root: string
  relPath: string
  cache: EditorCache
  /** Fired after a successful save (the git tab refreshes on it). */
  onSaved: () => void
  /** Dirty-flag transitions — the page pins the tab on first edit. */
  onDirtyChange: (relPath: string, dirty: boolean) => void
}

export function EditorPane({
  host,
  root,
  relPath,
  cache,
  onSaved,
  onDirtyChange,
}: EditorPaneProps) {
  const absPath = joinPath(root, relPath)
  const [pane, setPane] = useState<PaneState>({ phase: 'loading' })
  const [draft, setDraftState] = useState('')
  const [savedContent, setSavedContent] = useState('')
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const seqRef = useRef(0)

  const entry = cache.get(relPath)

  useEffect(() => {
    const seq = ++seqRef.current
    setSaveError(null)
    const cached = cache.get(relPath)
    if (cached) {
      setDraftState(cached.draft)
      setSavedContent(cached.savedContent)
      setPane({ phase: 'ready' })
      return
    }
    setPane({ phase: 'loading' })
    coderReadFile(host, absPath)
      .then((out) => {
        if (seqRef.current !== seq) return
        const content = out.content ?? ''
        const fresh: EditorCacheEntry = {
          savedContent: content,
          draft: content,
          readOnly:
            out.is_utf8 === false
              ? 'binary'
              : out.more_lines
                ? 'truncated'
                : null,
          mode: out.mode ?? null,
          size: out.size ?? null,
        }
        cache.set(relPath, fresh)
        setDraftState(content)
        setSavedContent(content)
        setPane({ phase: 'ready' })
      })
      .catch((err: unknown) => {
        if (seqRef.current !== seq) return
        setPane({
          phase: 'error',
          message: errorMessage(err),
        })
      })
  }, [host, absPath, relPath, cache])

  const dirty = pane.phase === 'ready' && draft !== savedContent
  const readOnly = entry?.readOnly ?? null
  const canSave = pane.phase === 'ready' && readOnly === null && dirty && !saving

  const setDraft = useCallback(
    (next: string) => {
      setDraftState(next)
      const current = cache.get(relPath)
      if (current) {
        const wasDirty = current.draft !== current.savedContent
        current.draft = next
        const isDirty = next !== current.savedContent
        if (wasDirty !== isDirty) onDirtyChange(relPath, isDirty)
      }
    },
    [cache, relPath, onDirtyChange],
  )

  const save = useCallback(() => {
    const current = cache.get(relPath)
    if (!current || current.readOnly !== null || saving) return
    if (current.draft === current.savedContent) return
    const body = current.draft
    setSaving(true)
    setSaveError(null)
    coderWriteFile(host, absPath, body, current.mode)
      .then((result) => {
        if (!result.success) {
          setSaveError(result.error?.message ?? 'save failed')
          return
        }
        current.savedContent = body
        setSavedContent(body)
        onDirtyChange(relPath, current.draft !== body)
        onSaved()
      })
      .catch((err: unknown) => {
        setSaveError(errorMessage(err))
      })
      .finally(() => setSaving(false))
  }, [host, absPath, relPath, cache, saving, onSaved, onDirtyChange])

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') {
        e.preventDefault()
        save()
      }
    },
    [save],
  )

  return (
    <div className="shui-main-pane">
      <div className="shui-editor-head">
        <span className="path" title={absPath}>
          {relPath}
        </span>
        {dirty ? <span className="shui-dirty" title="unsaved changes" /> : null}
        {readOnly ? (
          <span className="shui-ro-note">
            {readOnly === 'binary'
              ? 'binary — read-only'
              : 'truncated read — read-only'}
          </span>
        ) : null}
        {entry?.size != null ? (
          <span className="meta">{formatBytes(entry.size)}</span>
        ) : null}
        <span className="spacer" />
        {saveError ? <span className="shui-ro-note warn">{saveError}</span> : null}
        <button
          type="button"
          className="shui-save-btn"
          disabled={!canSave}
          onClick={save}
          title="save (⌘S)"
        >
          {saving ? 'saving…' : 'save'}
        </button>
      </div>

      <div className="shui-editor-body">
        {pane.phase === 'loading' ? (
          <div className="shui-side-note">loading {relPath}…</div>
        ) : pane.phase === 'error' ? (
          <div className="shui-side-note warn">{pane.message}</div>
        ) : (
          <CodeEditor
            value={draft}
            onChange={setDraft}
            language={monacoLangFromPath(relPath)}
            readOnly={readOnly !== null}
            onKeyDown={onKeyDown}
            aria-label={`edit ${relPath}`}
            className="shui-editor"
          />
        )}
      </div>
    </div>
  )
}

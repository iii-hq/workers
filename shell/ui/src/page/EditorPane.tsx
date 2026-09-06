/* The main editing surface — the console's shared Monaco-backed
   CodeEditor over `coder::read-file` / `coder::create-file(overwrite)`.
   Never bundles an editor: Monaco ships once, inside the console.

   File content + unsaved drafts live in a page-owned cache keyed by
   path, so switching editor tabs never discards edits: the pane is
   remounted per file (key=path) and rehydrates from the cache instead
   of re-reading.

   Sizes: text reads carry an editor-sized budget (8 MiB) and the editor
   owns its viewport (`fill`), so a file of tens of thousands of lines
   renders only what is on screen; a file over the budget opens as a
   read-only window of its first lines. Raster images stream in bounded
   chunks into a Blob and never cross the socket as one frame. */

import {
  Button,
  CodeEditor,
  type CodeEditorHandle,
  type CodeEditorSelection,
  type Host,
  IconButton,
} from '@iii-dev/console-ui'
import { CircleAlert, Code, Eye, FileDiff, FileX, FolderOpen, Hash, MessageSquareQuote, RefreshCw, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage, formatBytes } from '../lib/format'
import { Breadcrumbs } from './Breadcrumbs'
import {
  coderReadFile,
  coderReadWindow,
  coderWriteFile,
  joinPath,
} from './coder'
import { readFileBytes } from './file-bytes'
import { ImagePreview } from './image-preview'
import {
  EDITOR_FULL_READ_BUDGET,
  IMAGE_PREVIEW_MAX_BYTES,
  isTooLargeError,
  LARGE_FILE_PREVIEW_LINES,
} from './large-file'
import { isMissingFileError, loadErrorMessage } from './load-error'
import { imageMimeFromPath, monacoLangFromPath } from './file-kinds'
import { PaneNotice } from './PaneNotice'
import { dirname } from './paths'
import { type LineRange, selectionLines } from './reference'
import { isRichPreviewPath, richPreviewNode } from './rich-preview'

export { imageMimeFromPath, monacoLangFromPath } from './file-kinds'

/** Read-only reasons: binary content, or a byte-capped read (saving a
    truncated body would destroy the tail). */
export type ReadOnlyReason = 'binary' | 'truncated' | null

export interface EditorCacheEntry {
  /** What the worker last gave (or accepted) for this file. */
  savedContent: string
  /** The live buffer, possibly unsaved. */
  draft: string
  /** Opaque digest returned by coder::read-file for optimistic saves. */
  revision?: string
  readOnly: ReadOnlyReason
  mode: number | null
  size: number | null
  /** Object (or data) URL when the file rendered as an image preview. */
  image?: string | null
  /** Set when only the first lines of a large file were read. */
  window?: { lineTo: number; totalLines: number | null }
}

/** Page-owned; survives tab switches, dropped when a tab closes. */
export type EditorCache = Map<string, EditorCacheEntry>

type PaneState =
  | { phase: 'loading'; progress?: { received: number; total: number } }
  /** `missing`: the worker cannot see the file (deleted or moved). */
  | { phase: 'error'; message: string; missing: boolean }
  | { phase: 'ready' }

interface EditorPaneProps {
  host: Host
  root: string
  rootLabel: string
  relPath: string
  cache: EditorCache
  /** Hands out object URLs for streamed images; the page revokes them. */
  createObjectUrl: (blob: Blob) => string
  /** Fired after a successful save (the git tab refreshes on it). */
  onSaved: () => void
  /** Dirty-flag transitions — the page pins the tab on first edit. */
  onDirtyChange: (relPath: string, dirty: boolean) => void
  /** Global review-pane preference; the header toggle overrides it per file. */
  richPreview?: boolean
  wordWrap?: boolean
  /** Land the cursor on a line — or select `line`..`endLine` — once the
      file is loaded; `seq` distinguishes repeated requests for the same line. */
  reveal?: { line: number; column?: number; endLine?: number; seq: number } | null
  /** Bumps when the page asks for the go-to-line box. */
  goToLineSeq?: number
  onRevealDir: (dir: string) => void
  onCompare: (relPath: string) => void
  /** The page's word on the file being gone from disk (the live feed, a
      probe after restore); a loaded buffer stays editable so a save can
      put the file back, and a failed load retries when it clears. */
  missing?: boolean
  /** What this pane's own read found out about the file. */
  onMissing?: (relPath: string, missing: boolean) => void
  /** Close this tab (the way out of a file that is gone). */
  onClose?: () => void
  /** Offer "Reference in chat" on a selection: the chosen lines go to the
      composer as a `#file(path:from-to)` mention. Absent = no offer. */
  onReferenceInChat?: (relPath: string, range: LineRange) => void
}

export function EditorPane({
  host,
  root,
  rootLabel,
  relPath,
  cache,
  createObjectUrl,
  richPreview = false,
  wordWrap = true,
  reveal = null,
  goToLineSeq = 0,
  onSaved,
  onDirtyChange,
  onRevealDir,
  onCompare,
  missing = false,
  onMissing,
  onClose,
  onReferenceInChat,
}: EditorPaneProps) {
  const absPath = joinPath(root, relPath)
  const editorRef = useRef<CodeEditorHandle>(null)
  const previewable = isRichPreviewPath(relPath)
  const [previewChoice, setPreviewChoice] = useState<boolean | null>(null)
  // biome-ignore lint/correctness/useExhaustiveDependencies: a changed global preference resets the per-file override
  useEffect(() => {
    setPreviewChoice(null)
  }, [richPreview])
  const showPreview = previewable && (previewChoice ?? richPreview)
  const [pane, setPane] = useState<PaneState>({ phase: 'loading' })
  const [draft, setDraftState] = useState('')
  const [savedContent, setSavedContent] = useState('')
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [gotoOpen, setGotoOpen] = useState(false)
  const [gotoValue, setGotoValue] = useState('')
  const gotoInputRef = useRef<HTMLInputElement>(null)
  const seqRef = useRef(0)
  // Bumped by "Try again" and by the file coming back: re-runs the load.
  const [loadAttempt, setLoadAttempt] = useState(0)
  const onMissingRef = useRef(onMissing)
  onMissingRef.current = onMissing

  const entry = cache.get(relPath)

  const failLoad = useCallback(
    (seq: number, err: unknown) => {
      if (seqRef.current !== seq) return
      const raw = errorMessage(err)
      const gone = isMissingFileError(raw)
      setPane({ phase: 'error', message: loadErrorMessage(raw), missing: gone })
      onMissingRef.current?.(relPath, gone)
    },
    [relPath],
  )

  // biome-ignore lint/correctness/useExhaustiveDependencies: loadAttempt re-runs the load on demand
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
    const mime = imageMimeFromPath(relPath)
    if (mime) {
      readFileBytes(host, absPath, mime, {
        maxBytes: IMAGE_PREVIEW_MAX_BYTES,
        onProgress: (received, total) => {
          if (seqRef.current === seq) setPane({ phase: 'loading', progress: { received, total } })
        },
      })
        .then((bytes) => {
          if (seqRef.current !== seq) return
          const fresh: EditorCacheEntry = {
            savedContent: '',
            draft: '',
            readOnly: 'binary',
            mode: null,
            size: bytes.size,
            image: createObjectUrl(bytes.blob),
          }
          cache.set(relPath, fresh)
          setDraftState('')
          setSavedContent('')
          setPane({ phase: 'ready' })
          onMissingRef.current?.(relPath, false)
        })
        .catch((err: unknown) => failLoad(seq, err))
      return
    }
    const finish = (fresh: EditorCacheEntry) => {
      if (seqRef.current !== seq) return
      cache.set(relPath, fresh)
      setDraftState(fresh.draft)
      setSavedContent(fresh.savedContent)
      setPane({ phase: 'ready' })
      onMissingRef.current?.(relPath, false)
    }
    coderReadFile(host, absPath, { maxOutputBytes: EDITOR_FULL_READ_BUDGET })
      .then((out) => {
        const content = out.content ?? ''
        finish({
          savedContent: content,
          draft: content,
          revision: out.revision ?? undefined,
          readOnly: out.is_utf8 === false ? 'binary' : out.more_lines ? 'truncated' : null,
          mode: out.mode ?? null,
          size: out.size ?? null,
        })
      })
      .catch(async (err: unknown) => {
        if (seqRef.current !== seq) return
        const message = errorMessage(err)
        if (!isTooLargeError(message)) {
          failLoad(seq, err)
          return
        }
        // Over the editor budget: the first lines, read-only.
        try {
          const out = await coderReadWindow(host, absPath, 1, LARGE_FILE_PREVIEW_LINES)
          const content = out.content ?? ''
          finish({
            savedContent: content,
            draft: content,
            readOnly: 'truncated',
            mode: out.mode ?? null,
            size: out.size ?? null,
            window: { lineTo: out.lines_returned ?? LARGE_FILE_PREVIEW_LINES, totalLines: out.total_lines ?? null },
          })
        } catch (windowError: unknown) {
          failLoad(seq, windowError)
        }
      })
  }, [host, absPath, relPath, cache, createObjectUrl, failLoad, loadAttempt])

  const retryLoad = useCallback(() => setLoadAttempt((attempt) => attempt + 1), [])
  // The file came back (the live feed saw it created) while this pane was
  // showing it gone: load it without being asked.
  useEffect(() => {
    if (!missing && pane.phase === 'error' && pane.missing) retryLoad()
  }, [missing, pane, retryLoad])

  const dirty = pane.phase === 'ready' && draft !== savedContent
  const readOnly = entry?.readOnly ?? null
  const ready = pane.phase === 'ready'
  useEffect(() => {
    if (!reveal || !ready) return
    const editor = editorRef.current
    if (!editor) return
    // A referenced range is selected so the lines read as the citation
    // they are; a console that predates `revealLines` lands on the line.
    if (reveal.endLine !== undefined && reveal.endLine > reveal.line && editor.revealLines) {
      editor.revealLines(reveal.line, reveal.endLine)
    } else {
      editor.revealLine(reveal.line, reveal.column)
    }
  }, [reveal, ready])
  const selectionActions = useMemo(
    () =>
      onReferenceInChat
        ? [
            {
              id: 'reference-in-chat',
              label: 'Reference in chat',
              icon: <MessageSquareQuote aria-hidden />,
              run: (selection: CodeEditorSelection) => onReferenceInChat(relPath, selectionLines(selection)),
            },
          ]
        : undefined,
    [onReferenceInChat, relPath],
  )
  useEffect(() => {
    if (goToLineSeq === 0) return
    setGotoOpen(true)
    window.requestAnimationFrame(() => gotoInputRef.current?.select())
  }, [goToLineSeq])
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
    if (current.revision === undefined && !missing) {
      setSaveError('reload the file before saving so its revision can be verified')
      return
    }
    const body = current.draft
    setSaving(true)
    setSaveError(null)
    // A file that is gone has no newer content to protect, and the worker
    // reports a missing file as a revision conflict — so the save that
    // puts it back goes without the precondition. (The live feed clears
    // `missing` when the file reappears, and the guard is back.)
    coderWriteFile(host, absPath, body, current.mode, missing ? undefined : current.revision)
      .then((result) => {
        if (!result.success) {
          setSaveError(result.error?.message ?? 'save failed')
          return
        }
        current.savedContent = body
        current.revision = result.revision ?? current.revision
        setSavedContent(body)
        onDirtyChange(relPath, current.draft !== body)
        onSaved()
      })
      .catch((err: unknown) => {
        setSaveError(errorMessage(err))
      })
      .finally(() => setSaving(false))
  }, [host, absPath, relPath, cache, saving, missing, onSaved, onDirtyChange])

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') {
        e.preventDefault()
        save()
      }
    },
    [save],
  )

  const submitGoto = () => {
    const line = Number.parseInt(gotoValue, 10)
    if (Number.isFinite(line) && line > 0) editorRef.current?.revealLine(line)
    setGotoOpen(false)
    setGotoValue('')
  }

  const lineCount = ready ? draft.split('\n').length : 0
  const loadingLabel =
    pane.phase === 'loading' && pane.progress
      ? `loading image ${formatBytes(pane.progress.received)} of ${formatBytes(pane.progress.total)}…`
      : `loading ${relPath}…`

  return (
    <div className="shui-main-pane">
      <div className="shui-editor-head">
        <Breadcrumbs path={relPath} rootLabel={rootLabel} onSelectDir={onRevealDir} />
        {dirty ? <span className="shui-dirty" title="unsaved changes" /> : null}
        {ready && missing ? (
          <span className="shui-ro-note warn" title="The file was deleted or moved on disk while this tab was open.">
            {readOnly === null ? 'gone from disk — save to put it back' : 'gone from disk'}
          </span>
        ) : null}
        {readOnly ? (
          <span className="shui-ro-note">
            {entry?.image
              ? 'image'
              : readOnly === 'binary'
                ? 'binary — read-only'
                : entry?.window
                  ? `large file — first ${entry.window.lineTo.toLocaleString()} of ${entry.window.totalLines?.toLocaleString() ?? '?'} lines, read-only`
                  : 'truncated read — read-only'}
          </span>
        ) : null}
        <span className="spacer" />
        {saveError ? <span className="shui-ro-note warn">{saveError}</span> : null}
        {entry?.size != null ? <span className="meta">{formatBytes(entry.size)}</span> : null}
        {ready && !entry?.image && !showPreview ? (
          gotoOpen ? (
            <form
              className="shui-goto"
              onSubmit={(event) => {
                event.preventDefault()
                submitGoto()
              }}
            >
              <input
                ref={gotoInputRef}
                type="text"
                inputMode="numeric"
                value={gotoValue}
                placeholder={`1–${lineCount}`}
                aria-label="Go to line"
                onChange={(event) => setGotoValue(event.target.value)}
                onBlur={() => setGotoOpen(false)}
                onKeyDown={(event) => {
                  if (event.key === 'Escape') {
                    event.preventDefault()
                    setGotoOpen(false)
                  }
                }}
                // biome-ignore lint/a11y/noAutofocus: the box exists because the user asked for it
                autoFocus
              />
            </form>
          ) : (
            <button
              type="button"
              className="shui-goto-btn"
              title="Go to line"
              onClick={() => {
                setGotoOpen(true)
              }}
            >
              <Hash aria-hidden />
              {lineCount.toLocaleString()} lines
            </button>
          )
        ) : null}
        {!entry?.image ? (
          <IconButton label="Compare with…" onClick={() => onCompare(relPath)}>
            <FileDiff aria-hidden />
          </IconButton>
        ) : null}
        {previewable ? (
          <IconButton
            label={showPreview ? 'Show source' : 'Show preview'}
            aria-pressed={showPreview}
            onClick={() => setPreviewChoice(!showPreview)}
          >
            {showPreview ? <Code aria-hidden /> : <Eye aria-hidden />}
          </IconButton>
        ) : null}
        {readOnly === null ? (
          <button type="button" className="shui-save-btn" disabled={!canSave} onClick={save} title="save (⌘S)">
            {saving ? 'saving…' : 'save'}
          </button>
        ) : null}
      </div>

      <div className="shui-editor-body" data-keybindings-standdown="">
        {pane.phase === 'loading' ? (
          <div className="shui-side-note">{loadingLabel}</div>
        ) : pane.phase === 'error' ? (
          <PaneNotice
            Icon={pane.missing ? FileX : CircleAlert}
            tone={pane.missing ? 'neutral' : 'warn'}
            title={pane.missing ? 'This file is no longer here' : 'This file could not be opened'}
            path={relPath}
            detail={
              pane.missing
                ? 'It was deleted or moved outside the editor. The tab stays until you close it, in case the file comes back.'
                : pane.message
            }
            actions={
              <>
                <Button type="button" variant="ghost" size="sm" onClick={retryLoad}>
                  <RefreshCw aria-hidden="true" />
                  Try again
                </Button>
                {dirname(relPath) !== '' ? (
                  <Button type="button" variant="ghost" size="sm" onClick={() => onRevealDir(dirname(relPath))}>
                    <FolderOpen aria-hidden="true" />
                    Show folder
                  </Button>
                ) : null}
                {onClose ? (
                  <Button type="button" variant="ghost" size="sm" onClick={onClose}>
                    <X aria-hidden="true" />
                    Close tab
                  </Button>
                ) : null}
              </>
            }
          />
        ) : entry?.image ? (
          <ImagePreview src={entry.image} name={relPath} description={entry.size != null ? formatBytes(entry.size) : undefined} />
        ) : showPreview ? (
          <div className="shui-editor-preview">{richPreviewNode(relPath, draft)}</div>
        ) : (
          <CodeEditor
            ref={editorRef}
            value={draft}
            onChange={setDraft}
            language={monacoLangFromPath(relPath)}
            readOnly={readOnly !== null}
            onKeyDown={onKeyDown}
            aria-label={`edit ${relPath}`}
            className="shui-editor"
            fill
            lineNumbers
            wordWrap={wordWrap}
            selectionActions={selectionActions}
          />
        )}
      </div>
    </div>
  )
}

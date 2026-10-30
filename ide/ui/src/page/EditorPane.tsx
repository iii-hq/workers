/* The main editing surface — the console's shared Monaco-backed
   CodeEditor over `coder::read-file` / `coder::create-file(overwrite)`.
   Never bundles an editor: Monaco ships once, inside the console.

   File content + unsaved drafts live in a page-owned cache keyed by
   path, so switching editor tabs never discards edits: the pane is
   remounted per file (key=path) and rehydrates from the cache instead
   of re-reading. A write to the open file refills it in place
   (`cacheEpoch`), the editor kept.

   Sizes: text reads carry an editor-sized budget (8 MiB) and the editor
   owns its viewport (`fill`), so a file of tens of thousands of lines
   renders only what is on screen; a file over the budget opens as a
   read-only window of its first lines. Raster images stream in bounded
   chunks into a Blob and never cross the socket as one frame.

   `RevisionPane` shows a file as a commit left it, read-only: nothing in
   it edits, saves or marks its tab dirty. */

import {
  Button,
  CodeEditor,
  type CodeEditorHandle,
  type CodeEditorQuickEdit,
  type CodeEditorSelection,
  type Host,
  IconButton,
  Tooltip,
  useConfirm,
} from '@iii-dev/console-ui'
import { CircleAlert, Code, Eye, FileCheck, FileDiff, FileX, FolderOpen, Hash, Lock, RefreshCw, X } from 'lucide-react'
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { copyText, errorMessage, formatBytes } from '@iii-dev/console-ui/format'
import { Breadcrumbs } from './Breadcrumbs'
import {
  coderReadFile,
  coderReadWindow,
  coderWriteFile,
  joinPath,
} from './coder'
import { loadRevisionFile } from './diff-load'
import { rememberedViewState, rememberViewState } from './editor-cache'
import { useEditorPrefs } from './editor-prefs'
import { readFileBytes } from './file-bytes'
import { referenceWarning } from './reference-warning'
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
import { isQuickOpenKey, quickOpenPlatform } from './quick-open'
import { resolveQuickEditModel, runQuickEdit } from './quick-edit'
import { modelLabel } from './use-commit-message'
import { formatFileReference, type LineRange, referenceLabel, selectionLines } from './reference'
import { isRichPreviewPath, richPreviewNode } from './rich-preview'
import { isSaveKey, saveKeyAria, saveKeyLabel } from './save-key'

export { imageMimeFromPath, monacoLangFromPath } from './file-kinds'

/** Read-only reasons: binary content, or a byte-capped read (saving a
    truncated body would destroy the tail). */
export type ReadOnlyReason = 'binary' | 'truncated' | null

/** Where "Add to Chat" put a reference: in a chat's composer, or nowhere,
    because several chats are open and the console would not guess. */
export type ReferenceDelivery = 'composed' | 'ambiguous'

/** How long a note about a reference stays over the code. */
const REFERENCE_NOTE_MS = 4000

const PLATFORM = quickOpenPlatform()

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
  /** User/source override survives tab remounts until the global preference changes. */
  preview?: { choice: boolean; preference: boolean }
  /** Set when only the first lines of a large file were read. */
  window?: { lineTo: number; totalLines: number | null }
}

/** Page-owned; survives tab switches, dropped when a tab closes. */
export type EditorCache = Map<string, EditorCacheEntry>

type PaneState =
  | { phase: 'loading'; progress?: { received: number; total: number } }
  /** `missing`: the worker cannot see the file (deleted or moved);
      `locked`: it is one of the protected paths, which it never reads. */
  | { phase: 'error'; message: string; missing: boolean; locked?: boolean }
  | { phase: 'ready' }

// Shared, so setting the phase it already has renders nothing.
const PANE_LOADING: PaneState = { phase: 'loading' }
const PANE_READY: PaneState = { phase: 'ready' }

interface EditorPaneProps {
  host: Host
  root: string
  rootLabel: string
  relPath: string
  cache: EditorCache
  /** Bumps when the page refreshed or dropped this file's cache entry (the
      watcher saw it written, a discard or revert put it back): the pane
      refills in place. A save needs no word: the watcher reports it. */
  cacheEpoch?: number
  /** Hands out object URLs for streamed images; the page revokes them. */
  createObjectUrl: (blob: Blob) => string
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
  /** Acknowledge a successfully handled request so remounts cannot replay it. */
  onRevealHandled?: (relPath: string, seq: number) => void
  /** A one-shot "open preview" request (the explorer's Open preview), applied
      once the file is loaded and acknowledged like a reveal. */
  previewRequest?: { seq: number } | null
  onPreviewHandled?: (relPath: string, seq: number) => void
  onRevealDir: (dir: string) => void
  onCompare: (relPath: string) => void
  /** The page's word on the file being gone from disk (the live feed, a
      probe after restore); a loaded buffer stays editable so a save can
      put the file back, and a failed load retries when it clears. */
  missing?: boolean
  /** What this pane's own read found out about the file. */
  onMissing?: (relPath: string, missing: boolean) => void
  /** One of the worker's protected paths (`.env`, keys): its read fails
      like a missing file's, and the pane says why instead. */
  protectedPath?: boolean
  /** Close this tab (the way out of a file that is gone). */
  onClose?: () => void
  /** Offer "Add to Chat" (⌘L) on a selection: the chosen lines' absolute
      `#file(path:from-to)` reference goes to the chat beside the IDE. When
      that is `ambiguous` the reference is copied instead. Absent = no offer
      (Copy Reference is always there). */
  onAddToChat?: (reference: string) => ReferenceDelivery
  /** The chat beside this pane: Quick Edit (⌘I) uses the model its
      composer has picked. */
  conversationId?: string | null
  /** Open the page's go-to-file overlay. The editor body reclaims its
      chord in the capture phase: Monaco binds Ctrl+P to cursor-up on a
      Mac and would cancel the event before the page's keys see it. */
  onQuickOpen?: () => void
}

/** Load and edit a cached file, consuming each explicit line reveal once. */
function EditorPaneView({
  host,
  root,
  rootLabel,
  relPath,
  cache,
  cacheEpoch = 0,
  createObjectUrl,
  richPreview = false,
  wordWrap = true,
  reveal = null,
  goToLineSeq = 0,
  onRevealHandled,
  previewRequest = null,
  onPreviewHandled,
  onDirtyChange,
  onRevealDir,
  onCompare,
  onQuickOpen,
  missing = false,
  onMissing,
  protectedPath = false,
  onClose,
  onAddToChat,
  conversationId,
}: EditorPaneProps) {
  const absPath = joinPath(root, relPath)
  const editorRef = useRef<CodeEditorHandle>(null)
  const previewable = isRichPreviewPath(relPath)
  const [previewChoice, setPreviewChoiceState] = useState<boolean | null>(() => {
    const saved = cache.get(relPath)?.preview
    return saved?.preference === richPreview ? saved.choice : null
  })
  const previousPreference = useRef(richPreview)
  useEffect(() => {
    if (previousPreference.current === richPreview) return
    previousPreference.current = richPreview
    setPreviewChoiceState(null)
    const cached = cache.get(relPath)
    if (cached) delete cached.preview
  }, [richPreview, cache, relPath])
  const setPreviewChoice = useCallback((choice: boolean) => {
    setPreviewChoiceState(choice)
    const cached = cache.get(relPath)
    if (cached) cached.preview = { choice, preference: richPreview }
  }, [cache, relPath, richPreview])
  const showPreview = previewable && (previewChoice ?? richPreview)
  const [citationWarning, setCitationWarning] = useState<string | null>(null)
  const handledRevealSeq = useRef<number | null>(null)
  // A cached file starts ready: switching back to its tab paints no
  // "loading…" first.
  const [pane, setPane] = useState<PaneState>(() => (cache.has(relPath) ? PANE_READY : PANE_LOADING))
  const [draft, setDraftState] = useState(() => cache.get(relPath)?.draft ?? '')
  const [savedContent, setSavedContent] = useState(() => cache.get(relPath)?.savedContent ?? '')
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
  const protectedRef = useRef(protectedPath)
  protectedRef.current = protectedPath

  const entry = cache.get(relPath)

  const failLoad = useCallback(
    (seq: number, err: unknown) => {
      if (seqRef.current !== seq) return
      const raw = errorMessage(err)
      const unreadable = isMissingFileError(raw)
      // A protected file is there; the worker just never reads it.
      const locked = unreadable && protectedRef.current
      const gone = unreadable && !locked
      setPane({ phase: 'error', message: loadErrorMessage(raw), missing: gone, locked })
      onMissingRef.current?.(relPath, gone)
    },
    [relPath],
  )

  // biome-ignore lint/correctness/useExhaustiveDependencies: loadAttempt and cacheEpoch re-run the load on demand
  useEffect(() => {
    const seq = ++seqRef.current
    const cancel = () => { seqRef.current += 1 }
    setSaveError(null)
    const cached = cache.get(relPath)
    if (cached) {
      setDraftState(cached.draft)
      setSavedContent(cached.savedContent)
      setPane(PANE_READY)
      return cancel
    }
    setPane(PANE_LOADING)
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
          setPane(PANE_READY)
          onMissingRef.current?.(relPath, false)
        })
        .catch((err: unknown) => failLoad(seq, err))
      return cancel
    }
    const finish = (fresh: EditorCacheEntry) => {
      if (seqRef.current !== seq) return
      cache.set(relPath, fresh)
      setDraftState(fresh.draft)
      setSavedContent(fresh.savedContent)
      setPane(PANE_READY)
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
    return cancel
  }, [host, absPath, relPath, cache, createObjectUrl, failLoad, loadAttempt, cacheEpoch])

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
    if (!reveal || !ready || handledRevealSeq.current === reveal.seq) return
    if (showPreview) {
      setPreviewChoice(false)
      return
    }
    if (readOnly !== 'binary') {
      const editor = editorRef.current
      if (!editor) return
      // A referenced range is selected; older consoles land on its first line.
      if (reveal.endLine !== undefined && reveal.endLine > reveal.line && editor.revealLines) {
        editor.revealLines(reveal.line, reveal.endLine)
      } else {
        editor.revealLine(reveal.line, reveal.column)
      }
    }
    handledRevealSeq.current = reveal.seq
    // Snapshot the warning now, not from the user's subsequent edits.
    setCitationWarning(referenceWarning(reveal, draft, readOnly))
    onRevealHandled?.(relPath, reveal.seq)
  }, [reveal, ready, readOnly, showPreview, draft, setPreviewChoice, onRevealHandled, relPath])
  // Applied once loaded, so the choice lands in the cache entry and survives
  // a tab switch; a failed read keeps the request until a retry succeeds.
  const handledPreviewSeq = useRef<number | null>(null)
  useEffect(() => {
    if (!previewRequest || !ready || handledPreviewSeq.current === previewRequest.seq) return
    handledPreviewSeq.current = previewRequest.seq
    if (previewable && readOnly !== 'binary') setPreviewChoice(true)
    onPreviewHandled?.(relPath, previewRequest.seq)
  }, [previewRequest, ready, previewable, readOnly, setPreviewChoice, onPreviewHandled, relPath])
  // A reference to the selected lines, for a chat: put there with Add to
  // Chat, or copied for the person to paste into the chat they choose.
  // What happened is said for a moment over the code.
  const [referenceNote, setReferenceNote] = useState<string | null>(null)
  useEffect(() => {
    if (referenceNote === null) return
    const timer = window.setTimeout(() => setReferenceNote(null), REFERENCE_NOTE_MS)
    return () => window.clearTimeout(timer)
  }, [referenceNote])
  const selectionActions = useMemo(() => {
    const reference = (range: LineRange) => formatFileReference(absPath, range)
    const copy = (range: LineRange, done: string) => {
      const where = referenceLabel(relPath, range)
      void copyText(reference(range)).then((copied) =>
        setReferenceNote(copied ? done.replace('%s', where) : `Couldn't copy ${where}. The browser blocked the clipboard.`),
      )
    }
    return [
      ...(onAddToChat
        ? [
            {
              id: 'reference-in-chat',
              label: 'Add to Chat',
              shortcut: 'Mod+L',
              run: (selection: CodeEditorSelection) => {
                const range = selectionLines(selection)
                if (onAddToChat(reference(range)) === 'ambiguous') {
                  copy(range, 'More than one chat is open, so %s was copied. Paste it into the one you mean.')
                }
              },
            },
          ]
        : []),
      {
        id: 'copy-reference',
        label: 'Copy Reference',
        run: (selection: CodeEditorSelection) => copy(selectionLines(selection), 'Copied %s. Paste it into any chat.'),
      },
    ]
  }, [onAddToChat, absPath, relPath])
  const { confirm, dialog: confirmDialog } = useConfirm()
  // Back to the same scroll and cursor after a tab switch or a remount.
  const rememberPlace = useCallback((state: unknown) => rememberViewState(absPath, state), [absPath])
  // Quick Edit (⌘I): the editor runs the prompt; this asks the model.
  const chatModel = host.chat?.composerModel?.(conversationId ?? null) ?? null
  const quickEdit = useMemo<CodeEditorQuickEdit>(
    () => ({
      shortcut: 'Mod+I',
      model: chatModel,
      modelLabel: chatModel ? modelLabel(chatModel) : null,
      run: async (request) =>
        runQuickEdit(host, request, relPath, request.model ?? (await resolveQuickEditModel(host, conversationId))),
    }),
    [host, relPath, conversationId, chatModel],
  )
  // The page's counter outlives this pane: a request made before it
  // mounted (in another file) is not this pane's to answer.
  const goToLineSeen = useRef(goToLineSeq)
  useEffect(() => {
    if (goToLineSeq === goToLineSeen.current) return
    goToLineSeen.current = goToLineSeq
    setGotoOpen(true)
    window.requestAnimationFrame(() => gotoInputRef.current?.select())
  }, [goToLineSeq])
  const canSave = pane.phase === 'ready' && readOnly === null && dirty && !saving
  // Size, line count and Save stay out of the header unless asked for (IDE settings).
  const [prefs] = useEditorPrefs()

  const setDraft = useCallback(
    (next: string) => {
      setCitationWarning(null)
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
      })
      .catch((err: unknown) => {
        setSaveError(errorMessage(err))
      })
      .finally(() => setSaving(false))
  }, [host, absPath, relPath, cache, saving, missing, onDirtyChange])

  // On the pane, not the editor: the editor's keys bubble here, and so do
  // the header's, so the chord also saves from the save icon itself.
  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (isSaveKey(e)) {
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

  // Counted, not split: a split allocates a string per line on every keystroke.
  const lineCount = useMemo(() => (ready ? countLines(draft) : 0), [ready, draft])
  // Rendered again only when the body or the file changes.
  const preview = useMemo(() => (showPreview ? richPreviewNode(relPath, draft) : null), [showPreview, relPath, draft])
  const loadingLabel =
    pane.phase === 'loading' && pane.progress
      ? `loading image ${formatBytes(pane.progress.received)} of ${formatBytes(pane.progress.total)}…`
      : `loading ${relPath}…`

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: the pane relays the save chord bubbling out of the editor and header
    <div className="shui-main-pane" onKeyDown={onKeyDown}>
      <div className="shui-editor-head">
        <Breadcrumbs path={relPath} rootLabel={rootLabel} onSelectDir={onRevealDir} />
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
        {prefs.showFileInfo && entry?.size != null ? <span className="meta">{formatBytes(entry.size)}</span> : null}
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
          ) : prefs.showFileInfo ? (
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
          ) : null
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
            onClick={() => {
              setCitationWarning(null)
              setPreviewChoice(!showPreview)
            }}
          >
            {showPreview ? <Code aria-hidden /> : <Eye aria-hidden />}
          </IconButton>
        ) : null}
        {/* Unsaved edits show as a Save button (a file with a check) instead
            of a bare dot; the setting keeps it up always. A click asks first,
            ⌘S saves straight away. */}
        {readOnly === null && (dirty || prefs.showSaveButton) ? (
          <Tooltip label={saving ? 'Saving…' : `Save (${saveKeyLabel(PLATFORM)})`}>
            <button
              type="button"
              className="shui-save-btn"
              data-dirty={dirty || undefined}
              aria-label="Save"
              aria-keyshortcuts={saveKeyAria(PLATFORM)}
              aria-busy={saving || undefined}
              disabled={!canSave}
              onClick={() => {
                void confirm({
                  title: `Save changes to ${relPath.slice(relPath.lastIndexOf('/') + 1)}?`,
                  description: 'Your edits replace the file on disk.',
                  confirmLabel: 'Save file',
                }).then((ok) => {
                  if (ok) save()
                })
              }}
            >
              <FileCheck aria-hidden />
            </button>
          </Tooltip>
        ) : null}
        {confirmDialog}
      </div>

      {citationWarning ? <div className="shui-side-note" role="status">{citationWarning}</div> : null}
      <div className="shui-editor-body" data-keybindings-standdown="" onKeyDownCapture={quickOpenCapture(onQuickOpen)}>
        {/* Always mounted, so a screen reader hears each note as it lands. */}
        <div className="shui-editor-toast-region" aria-live="polite" aria-atomic="true">
          {referenceNote ? (
            <div key={referenceNote} className="shui-editor-toast">
              {referenceNote}
            </div>
          ) : null}
        </div>
        {pane.phase === 'loading' ? (
          <div className="shui-side-note">{loadingLabel}</div>
        ) : pane.phase === 'error' ? (
          <PaneNotice
            Icon={pane.locked ? Lock : pane.missing ? FileX : CircleAlert}
            tone={pane.locked || pane.missing ? 'neutral' : 'warn'}
            title={
              pane.locked
                ? 'Protected file'
                : pane.missing
                  ? 'This file is no longer here'
                  : 'This file could not be opened'
            }
            path={relPath}
            detail={
              pane.locked
                ? "It matches the IDE's protected paths (Settings › IDE), which keep secrets such as .env files and keys out of reach, agents' included, so it is not opened here. The terminal can still read it."
                : pane.missing
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
          <div className="shui-editor-preview">{preview}</div>
        ) : (
          <CodeEditor
            ref={editorRef}
            value={draft}
            onChange={setDraft}
            language={monacoLangFromPath(relPath)}
            readOnly={readOnly !== null}
            aria-label={`edit ${relPath}`}
            className="shui-editor"
            fill
            lineNumbers
            wordWrap={wordWrap}
            fontSize={prefs.fontSize}
            contextMenu
            selectionActions={selectionActions}
            quickEdit={quickEdit}
            initialViewState={rememberedViewState(absPath)}
            onViewStateChange={rememberPlace}
          />
        )}
      </div>
    </div>
  )
}

/** Memoized: the page re-renders often, and this only when its props change. */
export const EditorPane = memo(EditorPaneView)

/** The editor body reclaims the go-to-file chord in the capture phase
    (see `onQuickOpen`). */
function quickOpenCapture(onQuickOpen: (() => void) | undefined) {
  return (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!onQuickOpen || !isQuickOpenKey(event, PLATFORM)) return
    event.preventDefault()
    event.stopPropagation()
    onQuickOpen()
  }
}

interface RevisionPaneProps {
  host: Host
  root: string
  rootLabel: string
  relPath: string
  sha: string
  /** The tab's id, which keys `cache`. */
  id: string
  /** Bodies already read, by tab id: a commit's file never changes, so a
      tab that comes forward again shows it at once. Page-owned. */
  cache: Map<string, string>
  wordWrap?: boolean
  onRevealDir: (dir: string) => void
  onClose: () => void
  onQuickOpen?: () => void
}

const ignoreEdit = () => {}

function RevisionPaneView({
  host,
  root,
  rootLabel,
  relPath,
  sha,
  id,
  cache,
  wordWrap = true,
  onRevealDir,
  onClose,
  onQuickOpen,
}: RevisionPaneProps) {
  const [read, setRead] = useState<{ body: string } | { error: string } | null>(() => {
    const body = cache.get(id)
    return body === undefined ? null : { body }
  })
  // Bumped by "Try again": re-runs the read.
  const [attempt, setAttempt] = useState(0)
  // biome-ignore lint/correctness/useExhaustiveDependencies: attempt re-runs the read on demand
  useEffect(() => {
    if (cache.has(id)) return
    let cancelled = false
    setRead(null)
    loadRevisionFile(host, root, relPath, sha)
      .then((body) => {
        if (cancelled) return
        cache.set(id, body)
        setRead({ body })
      })
      .catch((err: unknown) => {
        if (!cancelled) setRead({ error: errorMessage(err) })
      })
    return () => {
      cancelled = true
    }
  }, [host, root, relPath, sha, id, cache, attempt])

  const short = sha.slice(0, 7)
  const [prefs] = useEditorPrefs()
  return (
    <div className="shui-main-pane">
      <div className="shui-editor-head">
        <Breadcrumbs path={relPath} rootLabel={rootLabel} onSelectDir={onRevealDir} />
        <span className="shui-ro-note" title={sha}>
          as of {short}, read-only
        </span>
        <span className="spacer" />
      </div>
      <div className="shui-editor-body" data-keybindings-standdown="" onKeyDownCapture={quickOpenCapture(onQuickOpen)}>
        {read === null ? (
          <div className="shui-side-note">
            loading {relPath} as of {short}…
          </div>
        ) : 'error' in read && read.error === 'binary file' ? (
          // What `loadRevisionFile` says of bytes that are not text: no retry
          // reads them any other way.
          <PaneNotice Icon={FileX} title="Binary file: no text to show" path={relPath} />
        ) : 'error' in read ? (
          <PaneNotice
            Icon={CircleAlert}
            tone="warn"
            title="This version could not be opened"
            path={relPath}
            detail={read.error}
            actions={
              <>
                <Button type="button" variant="ghost" size="sm" onClick={() => setAttempt((value) => value + 1)}>
                  <RefreshCw aria-hidden="true" />
                  Try again
                </Button>
                <Button type="button" variant="ghost" size="sm" onClick={onClose}>
                  <X aria-hidden="true" />
                  Close tab
                </Button>
              </>
            }
          />
        ) : (
          <CodeEditor
            value={read.body}
            onChange={ignoreEdit}
            language={monacoLangFromPath(relPath)}
            readOnly
            aria-label={`${relPath} as of ${short}`}
            className="shui-editor"
            contextMenu
            fill
            lineNumbers
            wordWrap={wordWrap}
            fontSize={prefs.fontSize}
          />
        )}
      </div>
    </div>
  )
}

/** Memoized like the editor: the page re-renders often. */
export const RevisionPane = memo(RevisionPaneView)

function countLines(text: string): number {
  let lines = 1
  for (let at = text.indexOf('\n'); at !== -1; at = text.indexOf('\n', at + 1)) lines += 1
  return lines
}

import { $createParagraphNode, $getRoot, type LexicalEditor } from 'lexical'
import {
  ArrowUp,
  ChevronDown,
  ChevronUp,
  Loader2,
  MoreHorizontal,
  Square,
} from 'lucide-react'
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { PermissionModePicker } from '@/components/permissions/PermissionModePicker'
import { MOBILE_LAYOUT_QUERY, useMediaQuery } from '@/hooks/use-media-query'
import { attachmentsFromFiles } from '@/lib/attachments/from-files'
import type { PermissionMode } from '@/lib/backend/approval-settings'
import {
  onComposerAttach,
  onComposerFocusRequest,
  onComposerInsert,
  requestComposerFocus,
} from '@/lib/composer-insert'
import type { FileMentionRef } from '@/lib/file-mention-token'
import type { FileSearchFn } from '@/lib/file-search'
import type { FunctionEntry } from '@/lib/functions'
import { cn } from '@/lib/utils'
import type {
  Attachment,
  ModelId,
  ModelOption,
  ThinkingLevel,
} from '@/types/chat'
import { AttachmentButton } from './AttachmentButton'
import { AttachmentChip } from './AttachmentChip'
import { BankPicker } from './BankPicker'
import { ChatSettingsSheet } from './ChatSettingsSheet'
import { composerCardClass, toolbarIconButtonClass } from './composer-chrome'
import { DirectoryPicker, type WorktreePickerOptions } from './DirectoryPicker'
import { LexicalShell } from './LexicalShell'
import { $appendComposerText } from './lexical/composer-text'
import { ModelPicker } from './ModelPicker'
import { nextHistoryTarget } from './queue-history'
import { useFileDrop } from './use-file-drop'

export interface ComposerSubmitPayload {
  text: string
  attachments: Attachment[]
}

/** Round icon action button (send / queue / stop) at the composer's edge. */
const actionButtonClass = cn(
  'composer-action inline-flex size-11 shrink-0 items-center justify-center rounded-full sm:size-10',
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
  'disabled:pointer-events-none disabled:opacity-40',
)

/** Send/stop is armed: inverted ink fill (light chip in dark mode). */
const actionReadyClass = 'bg-ink text-bg hover:bg-ink/90'

/** Composer is empty: the action recedes into the surface. */
const actionIdleClass =
  'bg-surface text-ink-faint hover:bg-surface-hover hover:text-ink'

/** The chevron at the project strip's edge that folds the card away (phone only). */
const stripToggleClass = cn(
  'absolute top-0 right-1 flex size-8 items-center justify-center rounded-full text-ink-faint sm:hidden',
  'hover:bg-surface-hover hover:text-ink',
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
)

interface ComposerProps {
  model: ModelId | null
  modelOptions: ModelOption[]
  catalogLoading?: boolean
  /** Increment to open the visible model picker from an external CTA. */
  modelPickerOpenRequest?: number
  /** Agent profile owns model + effort for this session. */
  modelLocked?: boolean
  /**
   * Per-conversation permission mode (manual / auto / full). Owned by
   * the backend `approval_settings` scope; ChatView passes the loaded
   * value here. While loading, the picker disables.
   */
  permissionMode: PermissionMode
  permissionModeLoading?: boolean
  /**
   * Whether to render the manual/auto/full permission-mode picker. Hidden when
   * the optional approval-gate worker is absent (nothing to control). Defaults
   * to `true` so existing callers / Storybook keep the picker.
   */
  showPermissionMode?: boolean
  thinkingLevel: ThinkingLevel
  /** Show the per-session working-directory picker (real backend only). */
  showWorkingDir?: boolean
  workingDir?: string | null
  /** Show the memory bank picker (memory worker present, real backend). */
  showMemoryBank?: boolean
  /** This chat's memory bank; null = the worker's default bank. */
  memoryBank?: string | null
  onMemoryBankChange?: (next: string | null) => void
  /**
   * When true, the picker renders read-only. NOT set after the first send —
   * the working dir stays re-scopable mid-conversation (ChatView always
   * passes `false`); this exists for callers that want a genuinely locked
   * picker (e.g. an embedded/read-only surface).
   */
  workingDirLocked?: boolean
  /** Stale-dir validation failure; auto-opens the picker with the message. */
  workingDirError?: string | null
  /** Stack default folder — pinned "default" row in the picker. */
  defaultWorkingDir?: string | null
  /** Worktrees tab in the picker (real backend + worktree worker only). */
  worktreePicker?: WorktreePickerOptions
  onModelChange: (next: ModelId) => void
  onWorkingDirChange?: (next: string) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  onPermissionModeChange: (next: PermissionMode) => void
  onSubmit: (payload: ComposerSubmitPayload) => void
  onStop?: () => void
  /** A stop was requested and the server hasn't finalized the turn yet. */
  stopping?: boolean
  isStreaming?: boolean
  /**
   * When true, the editor stays unlocked while streaming: a submit queues the
   * message on the running turn (delivered when the stream ends) and the stop
   * button stays available. When false (mock backends), streaming locks the
   * editor as before.
   */
  queueWhileStreaming?: boolean
  /** External lock (e.g. harness not installed). Editor + send disabled. */
  blocked?: boolean
  /** Prevent sending without locking the editor (e.g. transcript hydration). */
  submitBlocked?: boolean
  /** Placeholder while `blocked` is true. */
  blockedPlaceholder?: string
  /**
   * Put the caret in the editor on mount. The caller decides, because only it
   * knows whether focus is welcome: on a touch device it raises the on-screen
   * keyboard over the conversation, which is worse than aiming once.
   */
  autoFocus?: boolean
  /** Initial editor content (applied once on mount). */
  initialContent?: (editor: LexicalEditor) => void
  /**
   * Plain-text sugar for `initialContent` (applied once on mount): seeds the
   * editor AND the internal text state, so a restored draft submits without
   * requiring a keystroke first. Ignored when `initialContent` is given.
   */
  initialText?: string
  /**
   * Live text of the user's draft, fired on every editor change EXCEPT while
   * a queued message is being browsed/edited (that text is not the draft).
   * Powers the per-session draft persistence.
   */
  onTextChange?: (text: string) => void
  /** Initial attachment chips (applied once on mount). */
  initialAttachments?: Attachment[]
  functionEntries?: FunctionEntry[]
  /**
   * File search under `workingDir` for the `@` / `#` menus. Absent (mock
   * backends, no shell) = the menus offer functions only.
   */
  searchFiles?: FileSearchFn
  /**
   * Open a mentioned file where it can be read (the shell explorer, on the
   * referenced lines) when its pill is clicked. Absent = clicking a pill
   * only selects it.
   */
  onOpenFileMention?: (ref: FileMentionRef) => void
  /**
   * Queued messages the composer can browse+edit with ↑/↓, oldest→newest.
   * Non-destructive: browsing just loads a message; the change is committed on
   * submit (see `onEditQueued`). When set alongside `onEditQueued`, ↑/↓ cycle.
   */
  queuedForEdit?: Array<{ id: string; text: string; attachments: Attachment[] }>
  /**
   * Submit while browsing a queued message: save the edit in place (preserving
   * its queue position) with the new text + attachments, or remove it when the
   * payload is `null` (submitting an emptied composer). Given its id.
   */
  onEditQueued?: (
    id: string,
    payload: { text: string; attachments: Attachment[] } | null,
  ) => void
  /** Which queued message is being browsed (`null` = live draft), for highlight. */
  onBrowseChange?: (id: string | null) => void
}

export function Composer({
  model,
  modelOptions,
  catalogLoading,
  modelPickerOpenRequest,
  modelLocked,
  permissionMode,
  permissionModeLoading,
  showPermissionMode = true,
  thinkingLevel,
  showWorkingDir,
  workingDir,
  showMemoryBank,
  memoryBank,
  onMemoryBankChange,
  workingDirLocked,
  workingDirError,
  defaultWorkingDir,
  worktreePicker,
  onModelChange,
  onWorkingDirChange,
  onThinkingLevelChange,
  onPermissionModeChange,
  onSubmit,
  onStop,
  stopping,
  isStreaming,
  queueWhileStreaming,
  blocked,
  submitBlocked,
  blockedPlaceholder = 'chat unavailable…',
  autoFocus,
  initialContent,
  initialText,
  onTextChange,
  initialAttachments,
  functionEntries,
  searchFiles,
  onOpenFileMention,
  queuedForEdit,
  onEditQueued,
  onBrowseChange,
}: ComposerProps) {
  const [attachments, setAttachments] = useState<Attachment[]>(
    initialAttachments ?? [],
  )
  const [clearToken, setClearToken] = useState(0)
  const [mobileSettingsOpen, setMobileSettingsOpen] = useState(false)
  // Phone only: the chevron on the project strip folds the whole card away,
  // leaving the strip as the session's one-line footer. Wider layouts ignore
  // the flag (the chevron is hidden there), so a fold made on a phone never
  // strands a resized window without a composer.
  const [collapsed, setCollapsed] = useState(false)
  const collapsedRef = useRef(false)
  const setFolded = useCallback((next: boolean) => {
    collapsedRef.current = next
    setCollapsed(next)
  }, [])
  const mobileLayout = useMediaQuery(MOBILE_LAYOUT_QUERY)
  const hasProjectStrip = Boolean(showWorkingDir && onWorkingDirChange)
  const cardHidden = collapsed && mobileLayout && hasProjectStrip
  const cardId = useId()
  const textRef = useRef(initialContent ? '' : (initialText ?? ''))
  /* Boolean mirror of "the editor holds text": the action button swaps on
     the empty↔non-empty transition, and state updates for an unchanged
     boolean bail out — so plain typing still never re-renders the tree. */
  const [hasText, setHasText] = useState(
    () => textRef.current.trim().length > 0,
  )

  // One-shot mount initializer: seed the editor with the restored draft text.
  // Runs inside Lexical's initial-state update, so $-functions apply directly.
  // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only initializer, matching LexicalShell's one-shot initialConfig semantics.
  const resolvedInitialContent = useMemo(() => {
    if (initialContent) return initialContent
    const text = initialText
    if (!text) return undefined
    return () => {
      const root = $getRoot()
      root.clear()
      const paragraph = $createParagraphNode()
      $appendComposerText(paragraph, text)
      root.append(paragraph)
    }
  }, [])

  // ↑/↓ browse the queued messages for editing. `browseId` is the message the
  // editor currently holds (null = a live draft). Navigation is non-destructive
  // — the message is removed from the queue only when the edit is submitted.
  // The ref mirror gates `onTextChange` synchronously: `setBrowse` runs before
  // the loaded text echoes back through the editor's change event, so browsed
  // queue text is never reported as the live draft.
  const [browseId, setBrowseId] = useState<string | null>(null)
  const browseIdRef = useRef<string | null>(null)
  const setBrowse = useCallback(
    (id: string | null) => {
      browseIdRef.current = id
      setBrowseId(id)
      onBrowseChange?.(id)
    },
    [onBrowseChange],
  )

  // Drop the browse cursor if the message it pointed at left the queue.
  useEffect(() => {
    if (browseId !== null && !queuedForEdit?.some((m) => m.id === browseId)) {
      setBrowse(null)
    }
  }, [queuedForEdit, browseId, setBrowse])

  // Apply the pure ↑/↓ decision (see queue-history): load the chosen message
  // (returning its text for LexicalShell to insert) or return null to let the
  // arrow move the caret.
  const handleHistoryNav = useCallback(
    (direction: 'up' | 'down'): string | null => {
      const result = nextHistoryTarget(
        queuedForEdit ?? [],
        browseId,
        textRef.current,
        direction,
      )
      if (result.kind === 'noop') return null
      setBrowse(result.target.id)
      setAttachments(result.target.attachments)
      textRef.current = result.target.text
      setHasText(result.target.text.trim().length > 0)
      return result.target.text
    },
    [queuedForEdit, browseId, setBrowse],
  )

  const inputDisabled = blocked || (isStreaming && !queueWhileStreaming)
  const submitDisabled = inputDisabled || submitBlocked
  // Turn options are frozen on the running turn; changing them mid-stream
  // would silently not apply, so the pickers stay locked while streaming.
  const optionsDisabled = isStreaming || blocked

  const handleSubmit = useCallback(() => {
    if (submitDisabled) return
    const text = textRef.current.trim()
    const empty = !text && attachments.length === 0
    // Editing a queued message: save it in place (or remove it when emptied)
    // instead of sending a new message. A blank live composer is a no-op.
    if (browseId !== null) {
      onEditQueued?.(browseId, empty ? null : { text, attachments })
      setBrowse(null)
    } else {
      if (empty) return
      onSubmit({ text, attachments })
    }
    textRef.current = ''
    setHasText(false)
    // The submitted text is no longer a draft; report the clear even if the
    // editor-clear update below is tag-filtered by the change plugin.
    onTextChange?.('')
    setAttachments([])
    setClearToken((t) => t + 1)
  }, [
    submitDisabled,
    attachments,
    onSubmit,
    browseId,
    onEditQueued,
    setBrowse,
    onTextChange,
  ])

  const handleAttach = useCallback(
    (next: Attachment[]) => {
      setAttachments((current) => [...current, ...next])
      // Chips land in the card; a fold would hide what was just added.
      if (collapsedRef.current) setFolded(false)
    },
    [setFolded],
  )

  const handleRemoveAttachment = useCallback((id: string) => {
    setAttachments((current) => current.filter((a) => a.id !== id))
  }, [])

  const attachFiles = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return
      handleAttach(await attachmentsFromFiles(files))
    },
    [handleAttach],
  )
  useEffect(
    () => onComposerAttach((files) => void attachFiles(files)),
    [attachFiles],
  )

  // A folded card can neither take the caret nor show what lands in it, so
  // anything reaching for the composer from outside unfolds it first. The
  // focus request is then replayed: the editor's own listener already ran
  // against a hidden node.
  const refocusAfterUnfoldRef = useRef(false)
  useEffect(() => {
    const unfold = () => {
      if (!collapsedRef.current) return
      refocusAfterUnfoldRef.current = true
      setFolded(false)
    }
    const offFocus = onComposerFocusRequest(unfold)
    const offInsert = onComposerInsert(unfold)
    return () => {
      offFocus()
      offInsert()
    }
  }, [setFolded])
  useEffect(() => {
    if (collapsed || !refocusAfterUnfoldRef.current) return
    refocusAfterUnfoldRef.current = false
    requestComposerFocus()
  }, [collapsed])

  // The drop zone is the whole chat pane, claimed in the capture phase — see
  // `use-file-drop`. A drop onto the transcript, where people actually let go
  // of a screenshot, lands here too, and the editor never gets to eat it.
  const shell = useRef<HTMLDivElement>(null)
  // The typeahead menus take the card's width and hang above it.
  const card = useRef<HTMLDivElement>(null)
  const dragging = useFileDrop({
    anchorRef: shell,
    disabled: Boolean(inputDisabled),
    onFiles: (files) => void attachFiles(files),
  })

  const renderActionButton = () =>
    isStreaming &&
    !(queueWhileStreaming && (hasText || attachments.length > 0)) ? (
      <button
        type="button"
        onClick={onStop}
        disabled={stopping}
        aria-label={stopping ? 'stopping' : 'stop generating'}
        className={cn(actionButtonClass, actionReadyClass)}
      >
        {stopping ? (
          <Loader2 aria-hidden className="size-4 shrink-0 animate-spin" />
        ) : (
          <Square aria-hidden className="size-4 shrink-0 fill-current" />
        )}
      </button>
    ) : (
      <button
        type="button"
        onClick={handleSubmit}
        disabled={submitDisabled}
        aria-label={isStreaming ? 'queue message' : 'send message'}
        className={cn(
          actionButtonClass,
          hasText || attachments.length > 0
            ? actionReadyClass
            : actionIdleClass,
        )}
      >
        <ArrowUp aria-hidden className="size-[22px] shrink-0" />
      </button>
    )

  return (
    <div
      ref={shell}
      data-composer-streaming={isStreaming ? 'true' : undefined}
      className={cn(
        'composer-shell relative',
        hasProjectStrip ? 'pt-8' : 'pt-0',
      )}
    >
      {showWorkingDir && onWorkingDirChange ? (
        /* The project strip: a folder bar inset 8px from the card's edges and
           tucked behind its top edge. The button is taller than the visible
           strip (its bottom padding hides under the card), so the label sits
           centred in what shows; folded, the card is gone and the strip
           rounds off on its own. Fill and ink are both nudged a few percent
           toward each other so the strip reads as a step behind the card in
           either theme; hover only nudges the fill further — no outline. */
        <div className="composer-project-tab absolute inset-x-2 top-0 z-0">
          <DirectoryPicker
            value={workingDir ?? null}
            onChange={onWorkingDirChange}
            locked={workingDirLocked}
            disabled={optionsDisabled}
            externalError={workingDirError}
            defaultDir={defaultWorkingDir}
            worktrees={worktreePicker}
            className={cn(
              'w-full [&>button]:w-full [&>button]:justify-start [&>button]:gap-2 [&>button]:bg-[color-mix(in_oklab,var(--color-panel-raised)_96%,var(--color-ink))] [&>button]:pr-12 [&>button]:pl-4 [&>button]:text-xs [&>button]:font-medium [&>button]:text-[color-mix(in_oklab,var(--color-ink)_80%,var(--color-panel-raised))] [&>button:hover]:bg-[color-mix(in_oklab,var(--color-panel-raised)_90%,var(--color-ink))] [&>button>svg]:size-4',
              cardHidden
                ? '[&>button]:h-8 [&>button]:rounded-xl [&>button]:shadow-lift'
                : '[&>button]:h-11 [&>button]:rounded-t-xl [&>button]:rounded-b-none [&>button]:pb-3',
            )}
          />
          <button
            type="button"
            onClick={() => setFolded(!collapsedRef.current)}
            aria-label={collapsed ? 'expand composer' : 'collapse composer'}
            aria-expanded={!collapsed}
            aria-controls={cardId}
            className={stripToggleClass}
          >
            <span
              className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
              aria-hidden="true"
            />
            {collapsed ? (
              <ChevronUp className="size-4 shrink-0" aria-hidden />
            ) : (
              <ChevronDown className="size-4 shrink-0" aria-hidden />
            )}
          </button>
        </div>
      ) : null}

      <div
        id={cardId}
        ref={card}
        hidden={cardHidden}
        className={cn(
          'composer-card relative z-10 transition-shadow',
          composerCardClass,
          dragging && 'ring-2 ring-rule-focus',
        )}
      >
        {dragging ? (
          <div className="flex items-center justify-center border-b border-rule-2 px-3 py-2 font-mono text-[12px] text-ink-faint">
            drop to attach
          </div>
        ) : null}

        {attachments.length > 0 ? (
          <div className="flex flex-wrap gap-2 border-b border-rule-2 p-3">
            {attachments.map((a) => (
              <AttachmentChip
                key={a.id}
                attachment={a}
                onRemove={handleRemoveAttachment}
              />
            ))}
          </div>
        ) : null}

        <div className="composer-editor-slot px-1 pt-1">
          <LexicalShell
            onChange={(text) => {
              textRef.current = text
              setHasText(text.trim().length > 0)
              if (browseIdRef.current === null) onTextChange?.(text)
            }}
            onSubmit={handleSubmit}
            clearToken={clearToken}
            placeholder={
              blocked
                ? blockedPlaceholder
                : isStreaming
                  ? queueWhileStreaming
                    ? 'queue a message…'
                    : 'streaming response…'
                  : 'send a message…'
            }
            disabled={inputDisabled}
            autoFocus={autoFocus}
            initialContent={resolvedInitialContent}
            functionEntries={functionEntries}
            searchFiles={searchFiles}
            onOpenFileMention={onOpenFileMention}
            menuFrameRef={card}
            onHistoryNav={onEditQueued ? handleHistoryNav : undefined}
          />
        </div>

        {/* Tapping a toolbar button must not take the caret: on a phone the
            blur folds the card to one line and drops the keyboard, so the
            tap would land on a surface that just moved. The pickers open on
            click, which a cancelled pointerdown leaves alone. */}
        <div
          className="composer-mobile-toolbar flex min-w-0 items-center gap-1 px-2 pb-2 sm:hidden"
          onPointerDown={(event) => event.preventDefault()}
        >
          <AttachmentButton
            onAttach={handleAttach}
            disabled={inputDisabled}
            className={toolbarIconButtonClass}
          />
          <button
            type="button"
            onClick={() => setMobileSettingsOpen(true)}
            aria-label="chat settings"
            className="relative flex size-12 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
          >
            <span
              className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
              aria-hidden="true"
            />
            <MoreHorizontal className="size-5 shrink-0" aria-hidden />
          </button>
          <ModelPicker
            value={model}
            options={modelOptions}
            openRequest={modelPickerOpenRequest}
            thinkingLevel={thinkingLevel}
            onChange={onModelChange}
            onThinkingLevelChange={onThinkingLevelChange}
            disabled={optionsDisabled || modelLocked}
            loading={catalogLoading}
            showRefresh={false}
            triggerAppearance="subtle"
            className="ml-auto min-w-0"
          />
          {renderActionButton()}
        </div>

        {/* Desktop toolbar: attach (and the optional memory / permission
            pickers) on the left; the quiet model label and the send button
            on the right, where the eye lands after typing. */}
        <div className="hidden min-w-0 items-center gap-1 px-2 pb-2 sm:flex">
          <div className="flex min-w-0 flex-1 items-center gap-1">
            <AttachmentButton
              onAttach={handleAttach}
              disabled={inputDisabled}
              className={toolbarIconButtonClass}
            />
            {showMemoryBank && onMemoryBankChange ? (
              <BankPicker
                value={memoryBank ?? null}
                onChange={onMemoryBankChange}
                disabled={optionsDisabled}
              />
            ) : null}
            {showPermissionMode ? (
              <PermissionModePicker
                value={permissionMode}
                onChange={onPermissionModeChange}
                disabled={optionsDisabled || !!permissionModeLoading}
              />
            ) : null}
          </div>

          <div className="flex min-w-0 shrink-0 items-center gap-1.5">
            <ModelPicker
              value={model}
              options={modelOptions}
              openRequest={modelPickerOpenRequest}
              thinkingLevel={thinkingLevel}
              onChange={onModelChange}
              onThinkingLevelChange={onThinkingLevelChange}
              disabled={optionsDisabled || modelLocked}
              loading={catalogLoading}
              showRefresh={false}
              triggerAppearance="subtle"
              className="min-w-0 max-w-[16rem]"
            />
            {/* ONE action button. Mid-stream the slot shows Stop, but the moment
                the composer holds queueable content (text or attachments) it
                flips to send — the editor advertises "queue a message…", and the
                click must queue it, not kill the turn. */}
            {renderActionButton()}
          </div>
        </div>
      </div>

      <ChatSettingsSheet
        open={mobileSettingsOpen}
        onOpenChange={setMobileSettingsOpen}
        model={model}
        modelOptions={modelOptions}
        catalogLoading={catalogLoading}
        permissionMode={permissionMode}
        permissionModeLoading={permissionModeLoading}
        showPermissionMode={showPermissionMode}
        thinkingLevel={thinkingLevel}
        showWorkingDir={showWorkingDir}
        workingDir={workingDir}
        showMemoryBank={showMemoryBank}
        memoryBank={memoryBank}
        workingDirLocked={workingDirLocked}
        workingDirError={workingDirError}
        defaultWorkingDir={defaultWorkingDir}
        worktreePicker={worktreePicker}
        disabled={optionsDisabled}
        modelDisabled={modelLocked}
        onModelChange={onModelChange}
        onMemoryBankChange={onMemoryBankChange}
        onWorkingDirChange={onWorkingDirChange}
        onThinkingLevelChange={onThinkingLevelChange}
        onPermissionModeChange={onPermissionModeChange}
      />
    </div>
  )
}

import { Download, Plus } from 'lucide-react'
import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { ConversationSidebar } from '@/components/sidebar/ConversationSidebar'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { IconButton } from '@/components/ui/IconButton'
import { PageHeader, PageSidebar } from '@/components/ui/PageChrome'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { useContainerNarrow } from '@/hooks/use-container-narrow'
import { useMediaQuery } from '@/hooks/use-media-query'
import { useConversationsCtx } from '@/lib/conversations-context'
import { errText } from '@/lib/errors'
import { getIiiClient } from '@/lib/iii-client'
import {
  canForceDelete,
  deletionCommandErrorMessage,
  deletionFailureMessage,
  SessionTreeDeletionError,
  type SessionTreeDeletionSnapshot,
} from '@/lib/sessions/delete-tree'
import {
  getRemovalPreview,
  type RemovalPreview,
} from '@/lib/sessions/removal-preview'
import type { PageCommandsApi, PanelSide } from '@/types/injectable-ui'
import { ChatView } from './ChatView'
import { ConversationLoadNotice } from './ConversationLoadNotice'
import { DeletionDiagnostics } from './DeletionDiagnostics'
import { ImportConversationsDialog } from './ImportConversationsDialog'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

export type ChatPanelDensity = 'route' | 'dock'

interface ChatPanelProps {
  density?: ChatPanelDensity
  /** Session pinned to this workspace pane; leaves the global chat untouched. */
  conversationId?: string
  /** Outer edge occupied by this pane in a split workspace. */
  panelSide?: PanelSide
  /** Close the hosting pane — the header's standard ✕ when present. */
  onRequestClose?: () => void
  /** The pane's command registrar; chat's keys and palette rows go through it. */
  commands?: PageCommandsApi
}

/**
 * Container width (px) below which the panel collapses to the drill-in
 * session-list ⇄ chat flow (same pattern as the directory worker's page).
 * Keeps split workspace columns useful while still collapsing genuinely
 * narrow containers. Phone viewports are handled separately so this logic
 * stays aligned with Tailwind's `sm` breakpoint.
 */
const NARROW_BELOW = 560
const MOBILE_VIEWPORT_QUERY = '(max-width: 639px)'

/**
 * The chat surface: conversation sidebar + active chat.
 *
 * Layout adapts both to the panel width (the console can host it in panes)
 * and to the phone breakpoint used by the surrounding UI. In narrow mode,
 * the session list is its own full-width page; opening a conversation swaps
 * it for the chat with a ← back button. The view defaults to the chat so
 * squeezing a pane mid-conversation never yanks the operator to the list.
 */
export function ChatPanel({
  density = 'route',
  conversationId,
  panelSide = 'left',
  onRequestClose,
  commands,
}: ChatPanelProps) {
  const {
    conversations,
    activeId,
    active,
    watchConversation,
    createNew,
    select,
    rename,
    remove,
    setModel,
    setThinkingLevel,
    setWorkingDir,
    appendMessage,
    updateMessage,
    compactConversation,
    backend,
    modelOptions,
    catalogLoading,
    connectionState,
    conversationsLoading,
    conversationsError,
    conversationLoadErrors,
    retryConversations,
    missingConversationIds,
  } = useConversationsCtx()
  const [importOpen, setImportOpen] = useState(false)
  const removalDescriptionId = useId()
  const forceWarningId = useId()
  const pinned = conversationId !== undefined
  const displayedConversation = pinned
    ? (conversations.find(
        (conversation) => conversation.id === conversationId,
      ) ?? null)
    : active
  const displayedId = pinned ? conversationId : activeId

  useEffect(() => {
    if (!displayedId) return
    return watchConversation(displayedId)
  }, [displayedId, watchConversation])

  const [rootRef, containerNarrow] = useContainerNarrow(NARROW_BELOW)
  const surfaceRef = useRef<HTMLDivElement | null>(null)
  const mobileViewport = useMediaQuery(MOBILE_VIEWPORT_QUERY)
  const narrow = containerNarrow || mobileViewport
  // Which page the narrow flow shows. Only consulted while narrow; kept
  // across resizes so widening and re-squeezing lands where you left off.
  const [narrowView, setNarrowView] = useState<'list' | 'chat'>('chat')
  // Snapshot the identity: server deletion events can remove the sidebar row
  // before the tree operation is terminal, without dismissing this dialog.
  const [pendingRemoval, setPendingRemoval] = useState<RemovalPreview | null>(
    null,
  )
  const [checkingRemoval, setCheckingRemoval] = useState(false)
  const [previewError, setPreviewError] = useState<{
    id: string
    message: string
  } | null>(null)
  const previewWaitRef = useRef<symbol | null>(null)
  const [removalPending, setRemovalPending] = useState(false)
  const [removalError, setRemovalError] = useState<string | null>(null)
  const [removalSnapshot, setRemovalSnapshot] =
    useState<SessionTreeDeletionSnapshot | null>(null)
  const [forceConfirmation, setForceConfirmation] = useState(false)
  const [forcePending, setForcePending] = useState(false)

  useEffect(() => {
    if (forceConfirmation) cancelRemovalRef.current?.focus()
  }, [forceConfirmation])
  const removalEpoch = useRef(0)
  const removalWaitRef = useRef<AbortController | null>(null)
  const cancelRemovalRef = useRef<HTMLButtonElement | null>(null)

  useEffect(
    () => () => {
      // Unmount only ends the browser subscription; it never stops a backend turn.
      previewWaitRef.current = null
      removalWaitRef.current?.abort()
      removalWaitRef.current = null
    },
    [],
  )

  // Header-level actions can create/select a conversation outside this
  // component. On phones, follow that new active id into the chat page.
  useEffect(() => {
    if (mobileViewport && displayedId) setNarrowView('chat')
  }, [displayedId, mobileViewport])

  const handleSelect = useCallback(
    (id: string) => {
      select(id)
      setNarrowView('chat')
    },
    [select],
  )

  // The sidebar's verbs, beside the chat's own: both live in the same pane.
  useEffect(() => {
    if (pinned) return
    return commands?.register([
      {
        id: 'new-chat',
        title: 'New chat',
        detail: 'Start a conversation',
        keywords: ['conversation', 'session', 'create'],
        run: () => {
          createNew()
          setNarrowView('chat')
        },
      },
      {
        id: 'search-chats',
        title: 'Search conversations',
        detail: 'Put the caret in the sidebar search',
        keywords: ['find', 'sessions', 'filter'],
        run: () => {
          setNarrowView('list')
          window.requestAnimationFrame(() => {
            surfaceRef.current
              ?.querySelector<HTMLElement>('[data-conversation-search]')
              ?.focus()
          })
        },
      },
    ])
  }, [commands, createNew, pinned])

  const handleCreate = useCallback(() => {
    createNew()
    setNarrowView('chat')
  }, [createNew])

  const handleBack = useCallback(() => {
    setNarrowView('list')
  }, [])

  const performRemoval = useCallback(
    async (
      target: RemovalPreview,
      silent = false,
      force?: { operation_id: string; attempt: number },
      recover = false,
      reviewOperationId?: string,
    ) => {
      if (removalWaitRef.current) return
      removalEpoch.current += 1
      const wait = new AbortController()
      removalWaitRef.current = wait
      setRemovalPending(true)
      setRemovalError(null)
      setRemovalSnapshot(null)
      setForcePending(!!force)
      setForceConfirmation(false)
      try {
        await remove(target.id, {
          signal: wait.signal,
          ...(force ? { force } : {}),
          ...(recover ? { recover } : {}),
          ...(reviewOperationId ? { reviewOperationId } : {}),
        })
        if (!wait.signal.aborted) setPendingRemoval(null)
      } catch (error) {
        if (!wait.signal.aborted) {
          // Empty chats skip confirmation, not error reporting or safe retry.
          if (silent) setPendingRemoval(target)
          setRemovalError(
            error instanceof SessionTreeDeletionError
              ? deletionFailureMessage(error.snapshot)
              : (deletionCommandErrorMessage(error, !!force) ??
                  `Completion is not confirmed. The backend may have continued. ${errText(error)}`),
          )
          setRemovalSnapshot(
            error instanceof SessionTreeDeletionError ? error.snapshot : null,
          )
        }
      } finally {
        if (removalWaitRef.current === wait) {
          removalWaitRef.current = null
          setRemovalPending(false)
          setForcePending(false)
        }
      }
    },
    [remove],
  )

  const requestRemoval = useCallback(
    async (id: string) => {
      if (removalWaitRef.current || previewWaitRef.current) return
      const conversation = conversations.find((item) => item.id === id)
      if (!conversation) {
        // Already gone (e.g. session::deleted): drop only this id's stale
        // preview error so its alert and Try again cannot stick around.
        setPreviewError((current) => (current?.id === id ? null : current))
        return
      }
      const request = Symbol(id)
      previewWaitRef.current = request
      setCheckingRemoval(true)
      setPreviewError(null)
      setRemovalError(null)
      setRemovalSnapshot(null)
      setForceConfirmation(false)
      try {
        const preview = await getRemovalPreview(conversation)
        if (previewWaitRef.current !== request) return
        previewWaitRef.current = null
        setCheckingRemoval(false)
        if (preview.empty) {
          await performRemoval(preview, true)
        } else {
          setPendingRemoval(preview)
        }
      } catch (error) {
        if (previewWaitRef.current !== request) return
        previewWaitRef.current = null
        setCheckingRemoval(false)
        setPreviewError({ id, message: errText(error) })
      }
    },
    [conversations, performRemoval],
  )

  const cancelRemoval = useCallback(() => {
    if (removalWaitRef.current) return
    removalEpoch.current += 1
    setPendingRemoval(null)
    setRemovalError(null)
    setRemovalSnapshot(null)
    setForceConfirmation(false)
  }, [])

  const confirmRemoval = useCallback(async () => {
    if (pendingRemoval)
      await performRemoval(
        pendingRemoval,
        false,
        removalSnapshot?.mode === 'force'
          ? {
              operation_id: removalSnapshot.operation_id,
              attempt: removalSnapshot.attempt,
            }
          : undefined,
        !removalSnapshot,
      )
  }, [pendingRemoval, performRemoval, removalSnapshot])

  const removalIdentity = removalSnapshot
    ? JSON.stringify([removalSnapshot.operation_id, removalSnapshot.attempt])
    : null
  const latestRemovalSnapshot = useRef(removalSnapshot)
  latestRemovalSnapshot.current = removalSnapshot

  // Refresh blocked diagnostics from native lifecycle events, never polling.
  // Eligibility changes from our own read must not tear down/recreate this
  // subscription and issue another immediate read (busy status can oscillate).
  useEffect(() => {
    if (!pendingRemoval || !removalIdentity || removalPending) return
    const expected = latestRemovalSnapshot.current
    if (!expected) return
    const generation = removalEpoch.current
    let stopped = false
    let reading = false
    const cleanup: (() => void)[] = []
    void getIiiClient()
      .then((client) => {
        if (stopped) return
        const refresh = async () => {
          if (reading || stopped) return
          reading = true
          try {
            const fresh =
              await client.trigger<SessionTreeDeletionSnapshot | null>(
                'harness::delete-session-tree-status',
                { operation_id: expected.operation_id },
              )
            if (
              stopped ||
              generation !== removalEpoch.current ||
              !fresh ||
              fresh.session_id !== expected.session_id ||
              fresh.operation_id !== expected.operation_id ||
              fresh.attempt < expected.attempt
            )
              return
            if (fresh.status !== 'failed') {
              void performRemoval(pendingRemoval, false, undefined, true)
            } else {
              setRemovalSnapshot((current) =>
                JSON.stringify(current) === JSON.stringify(fresh)
                  ? current
                  : fresh,
              )
              setRemovalError(deletionFailureMessage(fresh))
              if (!canForceDelete(fresh)) setForceConfirmation(false)
            }
          } catch {
            if (!stopped) {
              setRemovalSnapshot((current) =>
                current ? { ...current, force_eligible: false } : null,
              )
              setForceConfirmation(false)
              setRemovalError(
                'Unable to refresh deletion status. Completion is not confirmed; the backend may have continued.',
              )
            }
          } finally {
            reading = false
          }
        }
        const handlerId = `iii::console::blocked_deletion_${crypto.randomUUID()}`
        cleanup.push(client.on(handlerId, () => refresh()))
        for (const sessionId of expected.remaining_session_ids ?? [
          expected.session_id,
        ]) {
          cleanup.push(
            client.registerTrigger({
              type: 'harness::turn-completed',
              function_id: `${handlerId}::${client.browserId}`,
              config: { session_id: sessionId },
            }),
          )
        }
        cleanup.push(
          client.registerTrigger({
            type: 'harness::session-tree-deletion',
            function_id: `${handlerId}::${client.browserId}`,
            config: { session_id: expected.session_id },
          }),
        )
        cleanup.push(
          client.addConnectionStateListener((connection) => {
            if (connection === 'connected') void refresh()
          }),
        )
        void refresh()
      })
      .catch(() => undefined)
    return () => {
      stopped = true
      for (const off of cleanup) off()
    }
  }, [pendingRemoval, removalIdentity, removalPending, performRemoval])

  const removalAction = pendingRemoval?.hasRunningWork
    ? 'Stop and delete'
    : 'Delete'
  const removalProgress = forcePending
    ? 'Force deleting…'
    : pendingRemoval?.hasRunningWork
      ? 'Stopping and deleting…'
      : 'Deleting…'

  // Narrow: one page at a time — the session list, or the open chat.
  // With no active conversation the list is the only meaningful page.
  const showList =
    !pinned && (!narrow || narrowView === 'list' || !displayedConversation)
  const showChat =
    pinned ||
    !narrow ||
    (narrowView === 'chat' && Boolean(displayedConversation))

  return (
    <div
      ref={(node) => {
        surfaceRef.current = node
        rootRef(node)
      }}
      className="chat-surface flex-1 flex flex-col min-h-0 min-w-0"
    >
      <ImportConversationsDialog
        open={importOpen}
        onOpenChange={setImportOpen}
        onImported={handleSelect}
      />
      <ConversationLoadNotice
        loading={conversationsLoading && connectionState === 'connected'}
        error={
          conversationsError ||
          (displayedId ? conversationLoadErrors?.[displayedId] : null)
        }
        onRetry={retryConversations}
      />
      <div
        className={`flex flex-1 min-h-0 min-w-0${panelSide === 'right' ? ' flex-row-reverse' : ''}`}
      >
        {showList ? (
          <PageSidebar
            // onboarding-conversations: tour anchor (workers/onboarding). Do not remove.
            className="onboarding-conversations"
            label="Conversations"
            side={panelSide}
            storageKey="console:chat:conversations"
            defaultWidth={220}
            minWidth={160}
            maxWidth={420}
            collapsible
            resizable
            narrow={narrow}
            header={
              <>
                <Button
                  type="button"
                  variant="primary"
                  size="md"
                  className="h-12 flex-1 justify-center px-3 font-sans text-base normal-case sm:h-9 sm:justify-start sm:text-sm"
                  onClick={handleCreate}
                >
                  <Plus className="size-4 shrink-0" aria-hidden />
                  New chat
                </Button>
                <IconButton
                  label="Import conversations"
                  className="max-sm:size-12"
                  onClick={() => setImportOpen(true)}
                >
                  <Download aria-hidden />
                </IconButton>
              </>
            }
            collapsedActions={
              <IconButton
                label="New chat"
                tooltipSide={panelSide === 'left' ? 'right' : 'left'}
                onClick={handleCreate}
                className="size-7"
              >
                <Plus aria-hidden />
              </IconButton>
            }
          >
            <ConversationSidebar
              conversations={conversations}
              activeId={activeId}
              narrow={narrow}
              onSelect={handleSelect}
              onRename={rename}
              onRemove={requestRemoval}
            />
          </PageSidebar>
        ) : null}

        {showChat && displayedConversation ? (
          <ChatView
            key={displayedConversation.id}
            conversation={displayedConversation}
            backend={backend}
            modelOptions={modelOptions}
            catalogLoading={catalogLoading}
            density={density}
            panelTitle={pinned ? displayedConversation.title : undefined}
            onRequestClose={onRequestClose}
            commands={commands}
            onBack={narrow && !pinned ? handleBack : undefined}
            onUpdateModel={setModel}
            onUpdateThinkingLevel={setThinkingLevel}
            onUpdateWorkingDir={setWorkingDir}
            onAppendMessage={appendMessage}
            onPatchMessage={updateMessage}
            onCompactConversation={compactConversation}
          />
        ) : !narrow || pinned ? (
          <section className="flex-1 flex flex-col min-w-0 min-h-0">
            <PageHeader title="Chat" onClose={onRequestClose} />
            <div className="flex-1 flex items-center justify-center">
              <div className="font-sans text-base text-ink-faint">
                {pinned
                  ? connectionState === 'connected'
                    ? conversationId &&
                      missingConversationIds.has(conversationId)
                      ? 'Conversation not found.'
                      : 'Loading conversation…'
                    : 'Waiting for the session connection…'
                  : 'No conversation selected.'}
              </div>
            </div>
          </section>
        ) : null}
      </div>
      {checkingRemoval || (removalPending && !pendingRemoval) ? (
        <StatusPanel
          role="status"
          headline={checkingRemoval ? 'Checking conversation…' : 'Deleting…'}
        />
      ) : null}
      {previewError ? (
        <StatusPanel
          variant="alert"
          role="alert"
          headline="Could not check this conversation."
          detail={previewError.message}
          action={
            <Button
              variant="ghost"
              onClick={() => void requestRemoval(previewError.id)}
            >
              Try again
            </Button>
          }
        />
      ) : null}
      <Dialog
        open={pendingRemoval !== null}
        onOpenChange={(open) => !open && cancelRemoval()}
      >
        <DialogContent
          data-chat-delete-dialog=""
          aria-busy={removalPending}
          aria-describedby={
            forceConfirmation
              ? `${removalDescriptionId} ${forceWarningId}`
              : removalDescriptionId
          }
          onOpenAutoFocus={(event) => {
            event.preventDefault()
            cancelRemovalRef.current?.focus()
          }}
          onEscapeKeyDown={(event) => {
            // Radix observes Escape in capture, before SearchField can clear
            // in bubble. Leave a non-empty filter's first Escape in this modal.
            if (
              removalPending ||
              (event.target instanceof HTMLInputElement &&
                event.target.type === 'search' &&
                event.target.value !== '')
            )
              event.preventDefault()
          }}
          onInteractOutside={(event) => {
            if (removalPending) event.preventDefault()
          }}
        >
          <DialogTitle className="pr-8">
            {forceConfirmation
              ? 'Force delete this chat?'
              : `${removalAction} conversation?`}
          </DialogTitle>
          <div data-chat-delete-body="">
            {forceConfirmation ? (
              <StatusPanel
                className="mt-2"
                variant="warn"
                id={forceWarningId}
                headline="Cancellation is best effort."
                detail="This chat and its subchats will be permanently deleted even without cancellation confirmation. External operations may continue; previous side effects are not undone."
              />
            ) : null}
            <DialogDescription
              className="mt-2 break-words leading-relaxed"
              data-chat-delete-description=""
              id={removalDescriptionId}
            >
              <span className="font-medium text-ink">
                “{pendingRemoval?.title}”
              </span>
              {pendingRemoval?.hasChildren
                ? ` and its ${pendingRemoval.descendantCount} subagent conversation${pendingRemoval.descendantCount === 1 ? '' : 's'} will be permanently deleted.`
                : ' will be permanently deleted.'}{' '}
              This cannot be undone.
              {pendingRemoval?.hasRunningWork || pendingRemoval?.parentId ? (
                <span className="mt-2 block">
                  {pendingRemoval.hasRunningWork
                    ? forceConfirmation
                      ? 'Running work will be cancelled on a best-effort basis. '
                      : 'Running work will be stopped first. '
                    : null}
                  {pendingRemoval.parentId
                    ? 'The parent will be notified, not stopped or deleted.'
                    : null}
                </span>
              ) : null}
            </DialogDescription>
            {removalPending ? (
              <StatusPanel
                className="mt-3"
                role="status"
                headline={removalProgress}
                detail="Waiting for confirmation. Closing the panel won't cancel deletion."
              />
            ) : null}
            {removalError ? (
              <StatusPanel
                className="mt-3"
                variant="alert"
                role="alert"
                headline="Deletion could not be confirmed."
                data-chat-delete-error=""
                detail={removalError}
              />
            ) : null}
            {removalSnapshot && !forceConfirmation ? (
              <DeletionDiagnostics
                key={`${removalSnapshot.operation_id}:${removalSnapshot.attempt}`}
                snapshot={removalSnapshot}
              />
            ) : null}
          </div>
          <div data-chat-delete-actions="">
            <Button
              ref={cancelRemovalRef}
              type="button"
              variant="ghost"
              disabled={removalPending}
              onClick={() =>
                forceConfirmation
                  ? setForceConfirmation(false)
                  : cancelRemoval()
              }
            >
              {forceConfirmation ? 'Back' : removalError ? 'Close' : 'Cancel'}
            </Button>
            {removalSnapshot?.existing_deletion && !forceConfirmation ? (
              <Button
                type="button"
                variant="ghost"
                disabled={removalPending}
                onClick={async () => {
                  const existing = removalSnapshot.existing_deletion
                  if (!existing) return
                  const conversation = conversations.find(
                    (item) => item.id === existing.session_id,
                  )
                  if (!conversation) {
                    setRemovalError(
                      'The existing operation’s chat is unavailable in this workspace. No deletion was started.',
                    )
                    return
                  }
                  const generation = removalEpoch.current
                  try {
                    const preview = await getRemovalPreview(conversation)
                    if (
                      generation !== removalEpoch.current ||
                      removalWaitRef.current
                    )
                      return
                    setPendingRemoval(preview)
                    select(existing.session_id)
                    await performRemoval(
                      preview,
                      false,
                      undefined,
                      false,
                      existing.operation_id,
                    )
                  } catch (error) {
                    setRemovalError(errText(error))
                  }
                }}
              >
                Review existing deletion
              </Button>
            ) : null}
            {removalSnapshot && !forceConfirmation ? (
              <Button
                type="button"
                variant="ghost"
                disabled={removalPending}
                onClick={() =>
                  pendingRemoval &&
                  performRemoval(
                    pendingRemoval,
                    false,
                    undefined,
                    false,
                    removalSnapshot.operation_id,
                  )
                }
              >
                Refresh status
              </Button>
            ) : null}
            <Button
              type="button"
              variant="primary"
              disabled={removalPending}
              data-force-delete={forceConfirmation ? '' : undefined}
              onClick={() => {
                if (
                  forceConfirmation &&
                  pendingRemoval &&
                  canForceDelete(removalSnapshot) &&
                  removalSnapshot
                ) {
                  void performRemoval(pendingRemoval, false, {
                    operation_id: removalSnapshot.operation_id,
                    attempt: removalSnapshot.attempt,
                  })
                } else void confirmRemoval()
              }}
            >
              {removalPending
                ? removalProgress
                : forceConfirmation
                  ? 'Force delete'
                  : removalSnapshot
                    ? removalSnapshot.mode === 'force'
                      ? 'Retry force delete'
                      : 'Retry delete'
                    : removalError
                      ? `Retry ${removalAction.toLowerCase()}`
                      : removalAction}
            </Button>
            {!forceConfirmation && canForceDelete(removalSnapshot) ? (
              <Button
                type="button"
                variant="ghost"
                disabled={removalPending}
                onClick={() => setForceConfirmation(true)}
              >
                Force delete…
              </Button>
            ) : null}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}

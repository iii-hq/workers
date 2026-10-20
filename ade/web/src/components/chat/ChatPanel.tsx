import { SquarePen } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { AddProjectDialog } from '@/components/chat/AddProjectDialog'
import {
  NewChatDialog,
  type NewChatDraft,
} from '@/components/chat/NewChatDialog'
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
import {
  endSideChat,
  isSideChat,
  markDisplayed,
  shownInAnyTab,
  sideChatOf,
  sideChatsLeftByClosedTabs,
  sideChatsMadeHere,
} from '@/lib/chat-aside'
import { onChatDialogRequest, onChatShownRequest } from '@/lib/chat-requests'
import { sendDraftWhenReady } from '@/lib/composer-insert'
import { useConversationsCtx } from '@/lib/conversations-context'
import { errText } from '@/lib/errors'
import {
  getRemovalPreview,
  type RemovalPreview,
} from '@/lib/sessions/removal-preview'
import type { PageCommandsApi, PanelSide } from '@/types/injectable-ui'
import { ChatView } from './ChatView'
import { ConversationLoadNotice } from './ConversationLoadNotice'
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
/** A side chat untouched this long ends (see the effect below)… */
const SIDE_CHAT_IDLE_MS = 2 * 60 * 60 * 1000
/** …checked this often while the ADE stays open. */
const SIDE_CHAT_SWEEP_MS = 5 * 60 * 1000
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

  // Side chats end, besides Discard: two hours untouched, with their main
  // chat (deleted here, in another tab or elsewhere: the list marks it
  // missing), and when every ADE tab of this browser is closed. Leaving a
  // conversation keeps them. One whose conversation a panel of this tab
  // shows is never idle.
  const conversationsRef = useRef(conversations)
  conversationsRef.current = conversations
  useEffect(
    () => (displayedId ? markDisplayed(displayedId) : undefined),
    [displayedId],
  )
  // The server's list, not the local draft the list boots with.
  const listed = !conversationsLoading && !conversationsError
  useEffect(() => {
    if (!listed) return
    // Never one still working, nor one any tab of the browser shows.
    const sweep = async () => {
      const cutoff = Date.now() - SIDE_CHAT_IDLE_MS
      const idle = conversationsRef.current.filter(
        (conversation) =>
          sideChatOf(conversation) !== null &&
          conversation.updatedAt < cutoff &&
          conversation.status !== 'working',
      )
      if (idle.length === 0) return
      const shown = await shownInAnyTab()
      for (const conversation of idle) {
        const main = sideChatOf(conversation)
        if (main !== null && !shown.has(main) && !shown.has(conversation.id)) {
          endSideChat(conversation.id, remove)
        }
      }
    }
    // A microtask: a pane mounted in the same commit has marked what it
    // shows by then.
    queueMicrotask(() => void sweep())
    const timer = window.setInterval(() => void sweep(), SIDE_CHAT_SWEEP_MS)
    return () => window.clearInterval(timer)
  }, [listed, remove])
  const [leftByClosedTabs, setLeftByClosedTabs] = useState<readonly string[]>(
    [],
  )
  useEffect(() => {
    void sideChatsLeftByClosedTabs().then(setLeftByClosedTabs)
  }, [])
  useEffect(() => {
    if (!listed) return
    for (const id of leftByClosedTabs) {
      const conversation = conversations.find((c) => c.id === id)
      if (!conversation) continue
      if (isSideChat(conversation)) endSideChat(id, remove)
      // Kept in another browser: not a side chat any more.
      else sideChatsMadeHere.delete(id)
    }
  }, [listed, leftByClosedTabs, conversations, remove])
  useEffect(() => {
    if (!missingConversationIds?.size) return
    for (const conversation of conversations) {
      const main = sideChatOf(conversation)
      if (main !== null && missingConversationIds.has(main)) {
        endSideChat(conversation.id, remove)
      }
    }
  }, [conversations, missingConversationIds, remove])

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

  // An outside request to show a chat (a setup wizard example, a page that
  // started a turn, a @session pill) also moves a narrow desktop pane off its
  // list. A new active id alone does not: deleting the open chat from the list
  // keeps the list, and so does a resize.
  useEffect(
    () =>
      pinned ? undefined : onChatShownRequest(() => setNarrowView('chat')),
    [pinned],
  )

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
    ])
  }, [commands, createNew, pinned])

  // New chat is a dialog (project, first message, model), so the chat is
  // born with its first message on the way rather than as an empty draft.
  const [newChatOpen, setNewChatOpen] = useState(false)
  const handleNewChat = useCallback(
    (draft: NewChatDraft) => {
      const id = createNew({ text: draft.text })
      if (draft.model) setModel(id, draft.model)
      setThinkingLevel(id, draft.thinkingLevel)
      if (draft.workingDir) setWorkingDir(id, draft.workingDir)
      sendDraftWhenReady(id, draft.text)
      setNarrowView('chat')
    },
    [createNew, setModel, setThinkingLevel, setWorkingDir],
  )

  // Add project: a new chat scoped to the folder just added, so the first
  // message lands where the user pointed.
  const [addProjectOpen, setAddProjectOpen] = useState(false)
  const handleProjectAdded = useCallback(
    (path: string) => {
      const id = createNew()
      setWorkingDir(id, path)
      setNarrowView('chat')
    },
    [createNew, setWorkingDir],
  )

  // The shortcuts reach the pane through the request bus (App dispatches
  // them); a pinned pane leaves the global chat's dialogs alone.
  useEffect(
    () =>
      pinned
        ? undefined
        : onChatDialogRequest((dialog) => {
            if (dialog === 'new-chat') setNewChatOpen(true)
            else setAddProjectOpen(true)
          }),
    [pinned],
  )

  const handleBack = useCallback(() => {
    setNarrowView('list')
  }, [])

  const performRemoval = useCallback(
    async (target: RemovalPreview, silent = false) => {
      if (removalWaitRef.current) return
      const wait = new AbortController()
      removalWaitRef.current = wait
      setRemovalPending(true)
      setRemovalError(null)
      try {
        await remove(target.id, { signal: wait.signal })
        if (!wait.signal.aborted) setPendingRemoval(null)
      } catch (error) {
        if (!wait.signal.aborted) {
          // Empty chats skip confirmation, not error reporting or safe retry.
          if (silent) setPendingRemoval(target)
          setRemovalError(errText(error))
        }
      } finally {
        if (removalWaitRef.current === wait) {
          removalWaitRef.current = null
          setRemovalPending(false)
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
    setPendingRemoval(null)
    setRemovalError(null)
  }, [])

  const confirmRemoval = useCallback(async () => {
    if (pendingRemoval) await performRemoval(pendingRemoval)
  }, [pendingRemoval, performRemoval])

  const removalAction = pendingRemoval?.hasRunningWork
    ? 'Stop and delete'
    : 'Delete'
  const removalProgress = pendingRemoval?.hasRunningWork
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
      <AddProjectDialog
        open={addProjectOpen}
        onOpenChange={setAddProjectOpen}
        onAdded={handleProjectAdded}
      />
      <NewChatDialog
        open={newChatOpen}
        onOpenChange={setNewChatOpen}
        modelOptions={modelOptions}
        catalogLoading={catalogLoading}
        onCreate={handleNewChat}
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
            className="onboarding-conversations bg-white dark:bg-neutral-950"
            label="Conversations"
            side={panelSide}
            storageKey="console:chat:conversations"
            defaultWidth={220}
            minWidth={160}
            maxWidth={420}
            collapsible
            resizable
            narrow={narrow}
            /* The toggle leads the top row from the outer edge; New chat,
               search and import are navigation rows inside the sidebar. */
            togglePlacement="outer"
            header={<span className="sr-only">Conversations</span>}
            collapsedActions={
              <IconButton
                label="New chat"
                tooltipSide={panelSide === 'left' ? 'right' : 'left'}
                onClick={() => setNewChatOpen(true)}
                className="size-7"
              >
                <SquarePen aria-hidden />
              </IconButton>
            }
          >
            <ConversationSidebar
              conversations={conversations}
              activeId={activeId}
              narrow={narrow}
              onCreate={() => setNewChatOpen(true)}
              onAddProject={() => setAddProjectOpen(true)}
              onImport={() => setImportOpen(true)}
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
            asideSide={panelSide === 'right' ? 'left' : 'right'}
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
          onOpenAutoFocus={(event) => {
            event.preventDefault()
            cancelRemovalRef.current?.focus()
          }}
          onEscapeKeyDown={(event) => {
            if (removalPending) event.preventDefault()
          }}
          onInteractOutside={(event) => {
            if (removalPending) event.preventDefault()
          }}
        >
          <DialogTitle className="pr-8">
            {removalAction} conversation?
          </DialogTitle>
          <DialogDescription className="mt-2 break-words leading-relaxed">
            <span className="font-medium text-ink">
              “{pendingRemoval?.title}”
            </span>
            {pendingRemoval?.hasChildren
              ? ' and its subagent conversations will be permanently deleted.'
              : ' will be permanently deleted.'}{' '}
            This cannot be undone.
            {pendingRemoval?.hasRunningWork || pendingRemoval?.parentId ? (
              <span className="mt-2 block">
                {pendingRemoval.hasRunningWork
                  ? 'Running work will be stopped first. '
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
          <div data-chat-delete-actions="">
            <Button
              ref={cancelRemovalRef}
              type="button"
              variant="ghost"
              disabled={removalPending}
              onClick={cancelRemoval}
            >
              {removalError ? 'Close' : 'Cancel'}
            </Button>
            <Button
              type="button"
              variant="primary"
              disabled={removalPending}
              onClick={() => void confirmRemoval()}
            >
              {removalPending
                ? removalProgress
                : removalError
                  ? `Retry ${removalAction.toLowerCase()}`
                  : removalAction}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}

import { Plus } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { ConversationSidebar } from '@/components/sidebar/ConversationSidebar'
import { Button } from '@/components/ui/Button'
import { IconButton } from '@/components/ui/IconButton'
import { PageHeader, PageSidebar } from '@/components/ui/PageChrome'
import { useContainerNarrow } from '@/hooks/use-container-narrow'
import { useMediaQuery } from '@/hooks/use-media-query'
import { useConversationsCtx } from '@/lib/conversations-context'
import type { PageCommandsApi, PanelSide } from '@/types/injectable-ui'
import { ChatView } from './ChatView'

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
    missingConversationIds,
  } = useConversationsCtx()
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
        shortcut: 'N',
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
        shortcut: '/',
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
      className={`chat-surface flex-1 flex min-h-0 min-w-0${
        panelSide === 'right' ? ' flex-row-reverse' : ''
      }`}
    >
      {showList ? (
        <PageSidebar
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
            onRemove={remove}
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
                  ? conversationId && missingConversationIds.has(conversationId)
                    ? 'Conversation not found.'
                    : 'Loading conversation…'
                  : 'Waiting for the session connection…'
                : 'No conversation selected.'}
            </div>
          </div>
        </section>
      ) : null}
    </div>
  )
}

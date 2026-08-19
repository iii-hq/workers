import { Plus } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { ConversationSidebar } from '@/components/sidebar/ConversationSidebar'
import { Button } from '@/components/ui/Button'
import { IconButton } from '@/components/ui/IconButton'
import { PageHeader, PageSidebar } from '@/components/ui/PageChrome'
import { useContainerNarrow } from '@/hooks/use-container-narrow'
import { useMediaQuery } from '@/hooks/use-media-query'
import { useConversationsCtx } from '@/lib/conversations-context'
import type { PanelSide } from '@/types/injectable-ui'
import { ChatView } from './ChatView'

export type ChatPanelDensity = 'route' | 'dock'

interface ChatPanelProps {
  density?: ChatPanelDensity
  /** Outer edge occupied by this pane in a split workspace. */
  panelSide?: PanelSide
  /** Close the hosting pane — the header's standard ✕ when present. */
  onRequestClose?: () => void
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
  panelSide = 'left',
  onRequestClose,
}: ChatPanelProps) {
  const {
    conversations,
    activeId,
    active,
    createNew,
    select,
    rename,
    remove,
    setModel,
    setMode,
    setWorkingDir,
    appendMessage,
    updateMessage,
    compactConversation,
    backend,
    modelOptions,
    catalogLoading,
  } = useConversationsCtx()

  const [rootRef, containerNarrow] = useContainerNarrow(NARROW_BELOW)
  const mobileViewport = useMediaQuery(MOBILE_VIEWPORT_QUERY)
  const narrow = containerNarrow || mobileViewport
  // Which page the narrow flow shows. Only consulted while narrow; kept
  // across resizes so widening and re-squeezing lands where you left off.
  const [narrowView, setNarrowView] = useState<'list' | 'chat'>('chat')

  // Header-level actions can create/select a conversation outside this
  // component. On phones, follow that new active id into the chat page.
  useEffect(() => {
    if (mobileViewport && activeId) setNarrowView('chat')
  }, [activeId, mobileViewport])

  const handleSelect = useCallback(
    (id: string) => {
      select(id)
      setNarrowView('chat')
    },
    [select],
  )

  const handleCreate = useCallback(() => {
    createNew()
    setNarrowView('chat')
  }, [createNew])

  const handleBack = useCallback(() => {
    setNarrowView('list')
  }, [])

  // Narrow: one page at a time — the session list, or the open chat.
  // With no active conversation the list is the only meaningful page.
  const showList = !narrow || narrowView === 'list' || !active
  const showChat = !narrow || (narrowView === 'chat' && Boolean(active))

  return (
    <div
      ref={rootRef}
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

      {showChat && active ? (
        <ChatView
          key={active.id}
          conversation={active}
          backend={backend}
          modelOptions={modelOptions}
          catalogLoading={catalogLoading}
          density={density}
          onRequestClose={onRequestClose}
          onBack={narrow ? handleBack : undefined}
          onUpdateModel={setModel}
          onUpdateMode={setMode}
          onUpdateWorkingDir={setWorkingDir}
          onAppendMessage={appendMessage}
          onPatchMessage={updateMessage}
          onCompactConversation={compactConversation}
        />
      ) : !narrow ? (
        <section className="flex-1 flex flex-col min-w-0 min-h-0">
          <PageHeader title="Chat" onClose={onRequestClose} />
          <div className="flex-1 flex items-center justify-center">
            <div className="font-sans text-base text-ink-faint">
              No conversation selected.
            </div>
          </div>
        </section>
      ) : null}
    </div>
  )
}

import { useCallback, useEffect, useState } from 'react'
import { ConversationSidebar } from '@/components/sidebar/ConversationSidebar'
import { PageHeader } from '@/components/ui/PageChrome'
import { Prompt } from '@/components/ui/Prompt'
import { useContainerNarrow } from '@/hooks/use-container-narrow'
import { useSidebarWidth } from '@/hooks/use-sidebar-width'
import { useConversationsCtx } from '@/lib/conversations-context'
import { ChatView } from './ChatView'

export type ChatPanelDensity = 'route' | 'dock'

interface ChatPanelProps {
  density?: ChatPanelDensity
  /** Close the hosting pane — the header's standard ✕ when present. */
  onRequestClose?: () => void
}

const SIDEBAR_COLLAPSED_KEY = 'iii-chat-sidebar-collapsed'

/**
 * Container width (px) below which the panel collapses to the drill-in
 * session-list ⇄ chat flow (same pattern as the directory worker's page).
 * Tuned so half-screen workspace columns (~620px) keep the inline
 * sidebar; only genuinely narrow panes and phone viewports drill in.
 */
const NARROW_BELOW = 560

function loadSidebarCollapsed(): boolean {
  if (typeof window === 'undefined') return false
  try {
    return window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === '1'
  } catch {
    return false
  }
}

function persistSidebarCollapsed(value: boolean): void {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, value ? '1' : '0')
  } catch {
    // ignore quota / privacy-mode errors; collapse state is best-effort
  }
}

/**
 * The chat surface: conversation sidebar + active chat.
 *
 * Layout adapts to the width the panel HAS (a ResizeObserver on its own
 * root, not a viewport media query — the console can host it in panes of
 * any size). Under NARROW_BELOW px it becomes a drill-in flow: the
 * session list is its own full-width page, opening a conversation swaps
 * it for the chat with a ← back button. The view defaults to the chat so
 * squeezing a pane mid-conversation never yanks the operator to the list.
 */
export function ChatPanel({
  density = 'route',
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

  const [sidebarCollapsed, setSidebarCollapsed] =
    useState<boolean>(loadSidebarCollapsed)
  const { width: sidebarWidth, setWidth: setSidebarWidth } = useSidebarWidth()

  useEffect(() => {
    persistSidebarCollapsed(sidebarCollapsed)
  }, [sidebarCollapsed])

  const toggleSidebar = useCallback(() => {
    setSidebarCollapsed((v) => !v)
  }, [])

  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)
  // Which page the narrow flow shows. Only consulted while narrow; kept
  // across resizes so widening and re-squeezing lands where you left off.
  const [narrowView, setNarrowView] = useState<'list' | 'chat'>('chat')

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
    <div ref={rootRef} className="chat-surface flex-1 flex min-h-0 min-w-0">
      {showList ? (
        <ConversationSidebar
          conversations={conversations}
          activeId={activeId}
          collapsed={narrow ? false : sidebarCollapsed}
          onToggleCollapsed={narrow ? undefined : toggleSidebar}
          width={sidebarWidth}
          onWidthChange={narrow ? undefined : setSidebarWidth}
          narrow={narrow}
          onCreate={handleCreate}
          onSelect={handleSelect}
          onRename={rename}
          onRemove={remove}
        />
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
          <PageHeader title="chat" onClose={onRequestClose} />
          <div className="flex-1 flex items-center justify-center">
            <div className="font-mono text-[13px] text-ink-faint lowercase">
              <Prompt symbol="$">no conversation selected.</Prompt>
            </div>
          </div>
        </section>
      ) : null}
    </div>
  )
}

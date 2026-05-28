import { useCallback, useEffect, useState } from 'react'
import { ConversationSidebar } from '@/components/sidebar/ConversationSidebar'
import { Prompt } from '@/components/ui/Prompt'
import { useConversationsCtx } from '@/lib/conversations-context'
import { ChatView } from './ChatView'

export type ChatPanelDensity = 'route' | 'dock'

interface ChatPanelProps {
  density?: ChatPanelDensity
}

const SIDEBAR_COLLAPSED_KEY = 'iii-chat-sidebar-collapsed'

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

export function ChatPanel({ density = 'route' }: ChatPanelProps) {
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
    appendMessage,
    updateMessage,
    compactConversation,
    backend,
    modelOptions,
    catalogLoading,
  } = useConversationsCtx()

  const [sidebarCollapsed, setSidebarCollapsed] =
    useState<boolean>(loadSidebarCollapsed)

  useEffect(() => {
    persistSidebarCollapsed(sidebarCollapsed)
  }, [sidebarCollapsed])

  const toggleSidebar = useCallback(() => {
    setSidebarCollapsed((v) => !v)
  }, [])

  return (
    <div className="flex-1 flex min-h-0 min-w-0">
      <ConversationSidebar
        conversations={conversations}
        activeId={activeId}
        collapsed={sidebarCollapsed}
        onToggleCollapsed={toggleSidebar}
        density={density}
        onCreate={createNew}
        onSelect={select}
        onRename={rename}
        onRemove={remove}
      />

      {active ? (
        <ChatView
          key={active.id}
          conversation={active}
          backend={backend}
          modelOptions={modelOptions}
          catalogLoading={catalogLoading}
          density={density}
          onUpdateModel={setModel}
          onUpdateMode={setMode}
          onAppendMessage={appendMessage}
          onPatchMessage={updateMessage}
          onCompactConversation={compactConversation}
        />
      ) : (
        <section className="flex-1 flex items-center justify-center">
          <div className="font-mono text-[13px] text-ink-faint lowercase">
            <Prompt symbol="$">no conversation selected.</Prompt>
          </div>
        </section>
      )}
    </div>
  )
}

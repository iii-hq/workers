import { ChatView } from '@/components/chat/ChatView'
import { ConversationSidebar } from '@/components/sidebar/ConversationSidebar'
import { Prompt } from '@/components/ui/Prompt'
import { useConversations } from '@/hooks/use-conversations'
import { useModelPickerSource } from '@/hooks/use-model-picker-source'
import { getDefaultBackend } from '@/lib/backend'

const backend = getDefaultBackend()

export function Chat() {
  const { modelOptions, catalogKeys, catalogLoading } = useModelPickerSource(
    backend.id,
  )

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
  } = useConversations(catalogKeys, !catalogLoading)

  return (
    <div className="flex-1 flex min-h-0">
      <ConversationSidebar
        conversations={conversations}
        activeId={activeId}
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

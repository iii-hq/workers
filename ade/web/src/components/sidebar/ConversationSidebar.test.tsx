import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { Conversation } from '@/types/chat'
import { ConversationSidebar } from './ConversationSidebar'

function conversation(id: string, title: string): Conversation {
  return {
    id,
    title,
    model: null,
    messages: [],
    hydrated: true,
    createdAt: 1,
    updatedAt: 1,
  }
}

describe('ConversationSidebar rename affordance', () => {
  it('exposes the open chat row the palette command drives', () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <ConversationSidebar
          conversations={[conversation('c1', 'Frontend')]}
          activeId="c1"
          narrow
          onSelect={vi.fn()}
          onRename={vi.fn(async () => null)}
          onRemove={vi.fn()}
        />
      </TooltipProvider>,
    )

    // `rename-chat` reaches the editor through exactly this pair of
    // attributes: `[aria-current="page"] [data-conversation-rename]`.
    expect(html).toContain('aria-current="page"')
    expect(html).toContain('data-conversation-rename=""')
    expect(html).toContain('aria-label="rename Frontend"')
  })
})

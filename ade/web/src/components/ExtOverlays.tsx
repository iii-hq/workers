import { memo } from 'react'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { type RegisteredOverlay, useExtOverlays } from '@/lib/ui-slots'

/**
 * The `overlays` extension slot: floating surfaces workers render above the
 * workspace regardless of the page or tab showing (a live thumbnail of what
 * an agent is browsing, say). The layer covers the shell but lets pointer
 * events through; each overlay positions itself and opts back in. It sits
 * above panes and below every popup (dialogs, sheets, menus are `z-50`).
 * Each overlay is told the active chat, the way a page is.
 */
export function ExtOverlays() {
  const overlays = useExtOverlays()
  // The conversations context is a new object on every store change — each
  // streamed token included — so this layer re-renders with it. Only the
  // active id goes down, and each overlay is memoized on it, so the overlays
  // themselves re-render when the chat changes and not when it streams.
  const conversationId = useConversationsCtxOptional()?.activeId ?? null
  if (overlays.length === 0) return null
  return (
    <div className="pointer-events-none fixed inset-0 z-30" data-ext-overlays>
      {overlays.map((overlay) => (
        <OverlaySlot
          key={overlay.id}
          render={overlay.render}
          conversationId={conversationId}
        />
      ))}
    </div>
  )
}

const OverlaySlot = memo(function OverlaySlot({
  render: Overlay,
  conversationId,
}: {
  render: RegisteredOverlay['render']
  conversationId: string | null
}) {
  return <Overlay conversationId={conversationId} />
})

import { useExtOverlays } from '@/lib/ui-slots'

/**
 * The `overlays` extension slot: floating surfaces workers render above the
 * workspace regardless of the page or tab showing (a live thumbnail of what
 * an agent is browsing, say). The layer covers the shell but lets pointer
 * events through; each overlay positions itself and opts back in. It sits
 * above panes and below every popup (dialogs, sheets, menus are `z-50`).
 */
export function ExtOverlays() {
  const overlays = useExtOverlays()
  if (overlays.length === 0) return null
  return (
    <div className="pointer-events-none fixed inset-0 z-30" data-ext-overlays>
      {overlays.map((overlay) => {
        const Overlay = overlay.render
        return <Overlay key={overlay.id} />
      })}
    </div>
  )
}

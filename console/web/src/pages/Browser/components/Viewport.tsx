import type { BrowserScreenshot } from '@/lib/browser'
import { cn } from '@/lib/utils'

/**
 * The session viewport: the latest screenshot, clickable. A click maps from
 * displayed-image space to page-viewport space (the capture's
 * `details.width/height` against the rendered size) and forwards as
 * `browser::act {action:'click', x, y}`. In pick mode the page is in
 * inspect mode, so the same forwarded click resolves the pick instead.
 */

interface ViewportProps {
  shot: BrowserScreenshot | null
  loading: boolean
  error: string | null
  picking: boolean
  onClickAt: (x: number, y: number) => void
}

export function Viewport({
  shot,
  loading,
  error,
  picking,
  onClickAt,
}: ViewportProps) {
  const handleClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    if (!shot || shot.width <= 0 || shot.height <= 0) return
    const img = e.currentTarget.querySelector('img')
    if (!img) return
    const rect = img.getBoundingClientRect()
    if (rect.width <= 0 || rect.height <= 0) return
    const x = Math.round(((e.clientX - rect.left) / rect.width) * shot.width)
    const y = Math.round(((e.clientY - rect.top) / rect.height) * shot.height)
    if (x < 0 || y < 0 || x > shot.width || y > shot.height) return
    onClickAt(x, y)
  }

  return (
    <div className="flex-1 min-h-0 overflow-auto bg-panel p-3">
      {shot?.dataUrl ? (
        <button
          type="button"
          onClick={handleClick}
          aria-label={
            picking
              ? 'pick an element on the page'
              : 'click on the page at this position'
          }
          title={
            picking
              ? 'pick mode: click an element to send it to chat'
              : 'clicks forward to the page'
          }
          className={cn(
            'block mx-auto max-w-full cursor-crosshair border bg-bg p-0',
            picking ? 'border-accent' : 'border-rule',
          )}
        >
          <img
            src={shot.dataUrl}
            alt={`live capture of ${shot.url}`}
            draggable={false}
            className="block max-w-full h-auto select-none"
          />
        </button>
      ) : (
        <div className="h-full flex items-center justify-center">
          <p className="font-mono text-[12px] lowercase text-ink-ghost">
            {error
              ? `screenshot failed: ${error}`
              : loading
                ? 'capturing viewport...'
                : 'no capture yet'}
          </p>
        </div>
      )}
    </div>
  )
}

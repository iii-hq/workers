import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'

/** The overlay rides a portal to `document.body`, outside the injectable
    UI's style scope, so it is styled inline rather than from styles.css. */
const OVERLAY_STYLE: React.CSSProperties = {
  position: 'fixed',
  inset: 0,
  zIndex: 2147483000,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  background: 'rgba(8, 8, 8, 0.88)',
  cursor: 'zoom-out',
}

const IMAGE_STYLE: React.CSSProperties = {
  maxWidth: '95vw',
  maxHeight: '95vh',
  objectFit: 'contain',
  boxShadow: '0 8px 40px rgba(0, 0, 0, 0.6)',
}

/**
 * A chat screenshot that opens full screen on click and closes on a click
 * or Escape. Escape is caught in the capture phase and stopped, so closing
 * the image never also closes a dialog behind it or reaches a shortcut.
 */
export function ZoomableImage({
  src,
  alt,
  className,
}: {
  src: string
  alt: string
  className?: string
}) {
  const [open, setOpen] = useState(false)

  useEffect(() => {
    if (!open) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.stopPropagation()
      event.preventDefault()
      setOpen(false)
    }
    window.addEventListener('keydown', onKeyDown, true)
    return () => window.removeEventListener('keydown', onKeyDown, true)
  }, [open])

  return (
    <>
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: Enter is covered below */}
      <img
        src={src}
        alt={alt}
        className={className}
        loading="lazy"
        role="button"
        tabIndex={0}
        onClick={() => setOpen(true)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') setOpen(true)
        }}
      />
      {open
        ? createPortal(
            <div
              style={OVERLAY_STYLE}
              data-keybindings-standdown=""
              onClick={() => setOpen(false)}
              role="presentation"
            >
              <img src={src} alt={alt} style={IMAGE_STYLE} />
            </div>,
            document.body,
          )
        : null}
    </>
  )
}

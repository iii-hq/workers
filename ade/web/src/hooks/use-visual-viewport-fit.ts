import { type RefObject, useEffect } from 'react'
import { DESKTOP_POINTER_QUERY, useMediaQuery } from './use-media-query'

/**
 * Keep a full-height shell inside the visual viewport while a phone's
 * keyboard is up.
 *
 * iOS never resizes the layout viewport for the keyboard (Android only with
 * `interactive-widget=resizes-content`, set in index.html): `100dvh` keeps
 * the keyboard's share and the browser pans the visual viewport to reveal
 * the focused field instead, so a shell with the composer at its bottom
 * ends up half under the keys. Mirroring the visual viewport — its height,
 * and its pan as a translate — puts the shell's bottom edge right above the
 * keyboard, and the flex column inside (header, transcript, composer) lays
 * itself out for the room that is actually on screen.
 *
 * Styles go straight onto the node: `scroll` fires per frame while the
 * viewport pans, and a state update at the app's root would re-render
 * everything each time. Nothing is written while pinch-zoomed (that also
 * shrinks the visual viewport), and nothing runs on pointer layouts, which
 * have no keyboard to dodge.
 */
export function useVisualViewportFit(ref: RefObject<HTMLElement | null>) {
  const pointer = useMediaQuery(DESKTOP_POINTER_QUERY)

  useEffect(() => {
    const node = ref.current
    const viewport =
      typeof window === 'undefined' ? undefined : window.visualViewport
    if (pointer || !node || !viewport) return

    const clear = () => {
      node.style.removeProperty('height')
      node.style.removeProperty('min-height')
      node.style.removeProperty('transform')
    }
    const fit = () => {
      // The layout viewport is what `100dvh` resolves to; a visual viewport
      // no shorter than it means no keyboard, and the stylesheet's own
      // sizing takes back over.
      const layoutHeight = document.documentElement.clientHeight
      if (viewport.scale > 1.001 || viewport.height >= layoutHeight - 1) {
        clear()
        return
      }
      const height = `${viewport.height}px`
      node.style.height = height
      node.style.minHeight = height
      node.style.transform =
        viewport.pageTop > 0 ? `translateY(${viewport.pageTop}px)` : ''
    }

    fit()
    // `resize` covers the keyboard opening and rotation; `scroll` covers the
    // pan that follows a focused field.
    viewport.addEventListener('resize', fit)
    viewport.addEventListener('scroll', fit)
    return () => {
      viewport.removeEventListener('resize', fit)
      viewport.removeEventListener('scroll', fit)
      clear()
    }
  }, [pointer, ref])
}

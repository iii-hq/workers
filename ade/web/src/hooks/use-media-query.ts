import { useEffect, useState } from 'react'

/**
 * A pointer you aim, on a screen with room for a keyboard that is already
 * there. Width alone would call a tablet in landscape a desktop, and taking
 * focus there raises the on-screen keyboard over whatever you were reading.
 */
export const DESKTOP_POINTER_QUERY = '(hover: hover) and (pointer: fine)'

/** The single-column phone layout: everything below the `sm` breakpoint. */
export const MOBILE_LAYOUT_QUERY = '(max-width: 639px)'

/** The reader asked the OS for less motion: presence runs instantly. */
export const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)'

export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() =>
    typeof window === 'undefined' || typeof window.matchMedia !== 'function'
      ? false
      : window.matchMedia(query).matches,
  )

  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return
    const media = window.matchMedia(query)
    const update = () => setMatches(media.matches)
    update()
    media.addEventListener('change', update)
    return () => media.removeEventListener('change', update)
  }, [query])

  return matches
}

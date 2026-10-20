import { useCallback, useEffect, useState } from 'react'

/** The theme a surface renders in: what `html[data-theme]` says. */
export type Theme = 'light' | 'dark'
/** What the user asked for: a theme, or `system` to follow the OS. */
export type ThemePreference = Theme | 'system'

const KEY = 'iii-theme'
const DARK_QUERY = '(prefers-color-scheme: dark)'
const THEME_COLORS: Record<Theme, string> = {
  light: '#f5f5f5',
  dark: '#0a0a0a',
}

function readTheme(): Theme {
  if (typeof document === 'undefined') return 'light'
  const attr = document.documentElement.dataset.theme
  return attr === 'dark' ? 'dark' : 'light'
}

function systemTheme(): Theme {
  if (typeof window === 'undefined' || !window.matchMedia) return 'light'
  return window.matchMedia(DARK_QUERY).matches ? 'dark' : 'light'
}

/** The theme a preference renders as right now. */
export function resolveTheme(preference: ThemePreference): Theme {
  return preference === 'system' ? systemTheme() : preference
}

/** The stored preference; nothing stored means "follow the system", which
 *  is also what index.html paints before the app loads. */
export function readThemePreference(): ThemePreference {
  try {
    const stored = localStorage.getItem(KEY)
    return stored === 'dark' || stored === 'light' ? stored : 'system'
  } catch {
    return 'system'
  }
}

export function applyDocumentTheme(theme: Theme): void {
  const root = document.documentElement
  const color = THEME_COLORS[theme]
  root.dataset.theme = theme
  root.style.colorScheme = theme
  root.style.backgroundColor = color
  document
    .querySelector<HTMLMetaElement>('meta[name="theme-color"]')
    ?.setAttribute('content', color)
}

function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true
  )
}

/**
 * Paint a theme, crossfading the whole page when the browser can (the view
 * transition's timing lives in index.css) and the reader has not asked for
 * reduced motion; otherwise the swap is instant, as it always was.
 */
function paintTheme(theme: Theme): void {
  if (readTheme() === theme) return
  const transition =
    !prefersReducedMotion() &&
    typeof document.startViewTransition === 'function'
      ? document.startViewTransition.bind(document)
      : null
  if (transition) transition(() => applyDocumentTheme(theme))
  else applyDocumentTheme(theme)
}

/** Store a preference and paint the theme it resolves to. */
export function applyThemePreference(preference: ThemePreference): void {
  try {
    localStorage.setItem(KEY, preference)
  } catch {
    /* best-effort */
  }
  const theme = resolveTheme(preference)
  paintTheme(theme)
}

/**
 * The theme the page is rendering in, kept in sync with `html[data-theme]`
 * from whichever caller changes it (the header switch, settings, the
 * Storybook toolbar). The setter is a shorthand for choosing that theme
 * outright; `useThemePreference` is the one that knows about `system`.
 */
export function useTheme(): [Theme, (next: Theme) => void] {
  const [theme, setThemeState] = useState<Theme>(() => readTheme())

  useEffect(() => {
    const root = document.documentElement
    const sync = () => {
      const next = readTheme()
      setThemeState((prev) => (prev === next ? prev : next))
    }
    const observer = new MutationObserver(sync)
    observer.observe(root, {
      attributes: true,
      attributeFilter: ['data-theme'],
    })
    return () => observer.disconnect()
  }, [])

  const setTheme = useCallback((next: Theme) => {
    applyThemePreference(next)
    setThemeState(next)
  }, [])
  return [theme, setTheme]
}

/**
 * The user's preference — light, dark, or follow the system — and its
 * setter. While the preference is `system`, an OS theme change repaints
 * the page; once a theme is chosen outright, the OS no longer has a say.
 */
export function useThemePreference(): [
  ThemePreference,
  (next: ThemePreference) => void,
] {
  const [preference, setPreferenceState] = useState<ThemePreference>(() =>
    readThemePreference(),
  )

  useEffect(() => {
    if (preference !== 'system' || typeof window === 'undefined') return
    const media = window.matchMedia?.(DARK_QUERY)
    if (!media) return
    const follow = () => paintTheme(systemTheme())
    media.addEventListener('change', follow)
    return () => media.removeEventListener('change', follow)
  }, [preference])

  const setPreference = useCallback((next: ThemePreference) => {
    applyThemePreference(next)
    setPreferenceState(next)
  }, [])
  return [preference, setPreference]
}

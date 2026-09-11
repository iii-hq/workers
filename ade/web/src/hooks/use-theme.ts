import { useCallback, useEffect, useState } from 'react'

export type Theme = 'light' | 'dark'

const KEY = 'iii-theme'

function readTheme(): Theme {
  if (typeof document === 'undefined') return 'light'
  const attr = document.documentElement.dataset.theme
  return attr === 'dark' ? 'dark' : 'light'
}

export function useTheme(): [Theme, (next: Theme) => void] {
  const [theme, setThemeState] = useState<Theme>(() => readTheme())

  useEffect(() => {
    document.documentElement.dataset.theme = theme
    try {
      localStorage.setItem(KEY, theme)
    } catch {
      /* best-effort */
    }
  }, [theme])

  // Keep every hook instance in sync when another caller (App settings,
  // Storybook toolbar, etc.) updates `html[data-theme]` directly.
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

  const setTheme = useCallback((next: Theme) => setThemeState(next), [])
  return [theme, setTheme]
}

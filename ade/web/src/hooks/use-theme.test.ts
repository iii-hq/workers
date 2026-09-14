import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const hook = readFileSync(new URL('./use-theme.ts', import.meta.url), 'utf8')
const html = readFileSync(new URL('../../index.html', import.meta.url), 'utf8')

describe('installed app theme', () => {
  it('syncs theme-color, color-scheme and the root background on theme changes', () => {
    expect(hook).toContain("light: '#f2f0ed'")
    expect(hook).toContain("dark: '#0a0a0a'")
    expect(hook).toContain('root.style.colorScheme = theme')
    expect(hook).toContain('root.style.backgroundColor = color')
    expect(hook).toContain('\'meta[name="theme-color"]\'')
    expect(hook).toContain('applyDocumentTheme(theme)')
  })

  it('applies the stored or system theme before the first paint', () => {
    expect(html).toContain("localStorage.getItem('iii-theme')")
    expect(html).toContain("matchMedia?.('(prefers-color-scheme: dark)')")
    expect(html).toContain('document.documentElement.style.colorScheme = theme')
    expect(html).toContain(
      'document.documentElement.style.backgroundColor = color',
    )
    expect(html).toContain('meta name="color-scheme" content="light dark"')
  })
})

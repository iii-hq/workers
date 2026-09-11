import { describe, expect, it } from 'vitest'
import { runtimeJsonSchema } from '../src/config.js'
import {
  DEFAULT_THEME,
  isThemeId,
  mix,
  readableOn,
  relativeLuminance,
  resolveTheme,
  THEMES,
  themeById,
} from '../src/themes.js'

describe('themes', () => {
  it('ships six distinct themes with a valid default', () => {
    expect(THEMES.length).toBe(6)
    expect(new Set(THEMES.map((theme) => theme.id)).size).toBe(6)
    expect(isThemeId(DEFAULT_THEME)).toBe(true)
    expect(themeById('nope').id).toBe(DEFAULT_THEME)
  })

  it('applies brand then deck overrides and recomputes contrast', () => {
    const resolved = resolveTheme(
      { theme: 'paper', theme_overrides: { accent: '#000000' } },
      { accent: '#ff0000', footer: 'ACME' },
    )
    expect(resolved.colors.accent).toBe('#000000')
    expect(resolved.colors.accent_ink).toBe('#ffffff')
    expect(resolved.footer).toBe('ACME')
    const dark = resolveTheme({ theme: 'paper', theme_overrides: { background: '#000000' } })
    expect(dark.dark).toBe(true)
    expect(dark.colors.surface).not.toBe('#ffffff')
  })

  it('ignores invalid colors', () => {
    expect(resolveTheme({ theme: 'slate', theme_overrides: { accent: 'red' } }).colors.accent).toBe(
      themeById('slate').colors.accent,
    )
  })

  it('has color helpers that behave', () => {
    expect(relativeLuminance('#ffffff')).toBeCloseTo(1)
    expect(relativeLuminance('#000')).toBe(0)
    expect(readableOn('#ffffff')).toBe('#111111')
    expect(mix('#000000', '#ffffff', 0.5)).toBe('#808080')
  })
})

describe('config schema', () => {
  it('publishes a JSON schema without $schema or engine_url', () => {
    const schema = runtimeJsonSchema()
    expect(schema.$schema).toBeUndefined()
    expect(Object.keys(schema.properties as Record<string, unknown>)).toEqual([
      'output_dir',
      'default_theme',
      'default_model',
      'default_provider',
      'outline_max_output_tokens',
      'brand_accent',
      'brand_footer',
    ])
  })
})

import { readFileSync } from 'node:fs'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Switch } from './Switch'

describe('Switch', () => {
  it('keeps native checkbox semantics and forwards form state', () => {
    const html = renderToStaticMarkup(
      <Switch
        aria-label="System notifications"
        name="system-notifications"
        defaultChecked
        disabled
      />,
    )

    expect(html).toContain('type="checkbox"')
    expect(html).toContain('role="switch"')
    expect(html).toContain('aria-label="System notifications"')
    expect(html).toContain('name="system-notifications"')
    expect(html).toContain('checked=""')
    expect(html).toContain('disabled=""')
    expect(html).toContain('iii-ui-switch__thumb')
  })

  it('styles browser-owned states and exposes a mobile touch target', () => {
    const css = readFileSync(
      new URL('../../styles/ui-recipes.css', import.meta.url),
      'utf8',
    )

    expect(css).toContain(':has(.iii-ui-switch__input:checked)')
    expect(css).toContain(':has(.iii-ui-switch__input:focus-visible)')
    expect(css).toContain(':has(.iii-ui-switch__input:disabled)')
    expect(css).toMatch(
      /@media \(max-width: 640px\)[\s\S]*?\.iii-ui-switch__input[\s\S]*?width: 3rem;[\s\S]*?height: 3rem;/,
    )
  })
})

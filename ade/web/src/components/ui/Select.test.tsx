import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Select } from './Select'

const options = [
  { value: 'one' as const, label: 'Option one' },
  { value: 'two' as const, label: 'Option two' },
]

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('Select', () => {
  it('uses the Radix combobox on desktop', () => {
    const html = renderToStaticMarkup(
      <Select
        value="one"
        options={options}
        onChange={() => {}}
        aria-label="Example"
      />,
    )

    expect(html).toContain('role="combobox"')
    expect(html).toContain('Option one')
  })

  it('forwards form identity and deep-link metadata to the desktop control', () => {
    const html = renderToStaticMarkup(
      <Select
        id="adapter"
        name="adapter.name"
        data-field="adapter.name"
        value="one"
        options={options}
        onChange={() => {}}
        aria-label="Example"
      />,
    )

    expect(html).toContain('id="adapter"')
    expect(html).toContain('name="adapter.name"')
    expect(html).toContain('data-field="adapter.name"')
  })

  it('preserves an unknown controlled value in form data', () => {
    const html = renderToStaticMarkup(
      <Select
        name="adapter.name"
        value="future"
        options={options}
        placeholder="Choose adapter"
        onChange={() => {}}
        aria-label="Example"
      />,
    )

    expect(html).toMatch(
      /<input(?=[^>]*name="adapter\.name")(?=[^>]*value="future")[^>]*>/,
    )
    expect(html).toContain('Choose adapter')
  })

  it('uses a dialog trigger on mobile', () => {
    vi.stubGlobal('window', {
      matchMedia: () => ({
        matches: true,
        addEventListener: () => {},
        removeEventListener: () => {},
      }),
    })

    const html = renderToStaticMarkup(
      <Select
        value="one"
        options={options}
        onChange={() => {}}
        aria-label="Example"
      />,
    )

    expect(html).toContain('aria-haspopup="dialog"')
    expect(html).toContain('aria-expanded="false"')
    expect(html).not.toContain('role="combobox"')
  })

  it('preserves form identity and deep-link metadata on mobile', () => {
    vi.stubGlobal('window', {
      matchMedia: () => ({
        matches: true,
        addEventListener: () => {},
        removeEventListener: () => {},
      }),
    })

    const html = renderToStaticMarkup(
      <Select
        id="adapter"
        name="adapter.name"
        data-field="adapter.name"
        value="one"
        options={options}
        onChange={() => {}}
        aria-label="Example"
      />,
    )

    expect(html).toContain('type="hidden"')
    expect(html).toContain('name="adapter.name"')
    expect(html).toContain('id="adapter"')
    expect(html).toContain('data-field="adapter.name"')
  })

  it('disables the mobile form value with its visible trigger', () => {
    vi.stubGlobal('window', {
      matchMedia: () => ({
        matches: true,
        addEventListener: () => {},
        removeEventListener: () => {},
      }),
    })

    const html = renderToStaticMarkup(
      <Select
        disabled
        name="adapter.name"
        value="one"
        options={options}
        onChange={() => {}}
        aria-label="Example"
      />,
    )

    expect(html).toMatch(
      /<input(?=[^>]*name="adapter\.name")(?=[^>]*disabled="")[^>]*>/,
    )
    expect(html).toMatch(/<button(?=[^>]*disabled="")[^>]*>/)
  })
})

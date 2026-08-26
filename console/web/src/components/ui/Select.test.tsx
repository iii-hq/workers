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
})

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Checkbox } from './Checkbox'

describe('Checkbox', () => {
  it('draws a dash, not a tick, when some but not all rows are picked', () => {
    // Sentinel's "select every group shown" with 1 of 7 picked read as
    // everything selected (QA 2026-11-16).
    const html = renderToStaticMarkup(
      <Checkbox aria-label="Select all" checked={false} indeterminate />,
    )
    expect(html).toContain('data-indeterminate')
    expect(html).toContain('lucide-minus')
    expect(html).toMatch(/lucide-check[^"]*group-data-indeterminate\/indicator:hidden/)
  })

  it('draws the tick when checked', () => {
    const html = renderToStaticMarkup(<Checkbox aria-label="Pick" checked />)
    expect(html).toContain('data-checked')
    expect(html).toContain('lucide-check')
  })
})

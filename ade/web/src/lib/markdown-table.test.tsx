import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Markdown } from './markdown'

describe('Markdown tables', () => {
  it('uses the shared responsive table visual in chat content', () => {
    const html = renderToStaticMarkup(
      <Markdown>{`| Field | Type | Notes |
| --- | --- | --- |
| \`name\` | string | Human-readable name. |`}</Markdown>,
    )

    expect(html).toContain('iii-ui-table-viewport')
    expect(html).toContain('iii-ui-table-frame')
    expect(html).toContain('iii-ui-table__header')
    expect(html).toContain('iii-ui-table__row')
    expect(html).toContain('iii-ui-table__head')
    expect(html).toContain('iii-ui-table__cell')
    expect(html).not.toContain('uppercase')
    expect(html).not.toContain('font-mono')
    expect(html).toContain('font-code')
  })
})

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

    // The shadcn table parts under the console's viewport and frame.
    expect(html).toContain('data-slot="table-container"')
    expect(html).toContain('rounded-lg border border-border')
    expect(html).toContain('data-slot="table-header"')
    expect(html).toContain('data-slot="table-row"')
    expect(html).toContain('data-slot="table-head"')
    expect(html).toContain('data-slot="table-cell"')
    expect(html).not.toContain('uppercase')
    expect(html).not.toContain('font-mono')
    expect(html).toContain('font-code')
  })
})

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

  it('lets a wide table scroll instead of clipping its last columns', () => {
    const html = renderToStaticMarkup(
      <Markdown>{`| Function | Description |
| --- | --- |
| shell::exec | Run a command in the foreground and return its full output |`}</Markdown>,
    )

    // The frame grows with the table, so the viewport has something to scroll.
    expect(html).toMatch(
      /<div [^>]*data-slot="table-frame"[^>]*class="[^"]*\binline-block\b[^"]*\bmin-w-full\b/,
    )
    // Cells wrap their text instead of forcing one line.
    expect(html).toMatch(
      /<td [^>]*class="[^"]*\bwhitespace-normal\b[^"]*"[^>]*>Run a command/,
    )
    expect(html).not.toContain('whitespace-nowrap')
  })
})

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'

// The real hook is supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui/hooks', () => ({ useSplitDrag: () => ({}) }))

import { DockPanel } from '../DockPanel'
import type { TerminalDock } from '../persist'

function render(dock: TerminalDock, narrow: boolean): string {
  return renderToStaticMarkup(
    <DockPanel dock={dock} size={300} narrow={narrow} label="Terminal" noun="terminal" onSizeChange={() => {}}>
      body
    </DockPanel>,
  )
}

describe('DockPanel', () => {
  it('sizes a right dock and offers its handle on a wide page', () => {
    const html = render('right', false)
    expect(html).toContain('width:300px')
    expect(html).toContain('role="separator"')
  })

  it('drops the size and the handle of a right dock on a narrow page', () => {
    const html = render('right', true)
    expect(html).not.toContain('style=')
    expect(html).not.toContain('role="separator"')
  })

  it('keeps a bottom dock resizable on a narrow page', () => {
    const html = render('bottom', true)
    expect(html).toContain('height:300px')
    expect(html).toContain('role="separator"')
  })
})

// The frame is the `shui-page` container: a rule in one of its own container
// queries never matches it, so the page sets its narrow layout by class.
it('styles.css restyles no workspace frame from inside its own container query', async () => {
  // Typed by hand: the worker UI tsconfig carries no Node types.
  const fs: { readFileSync(path: URL, encoding: 'utf8'): string } = await import('node:fs' as string)
  const css = fs.readFileSync(new URL('../../../styles.css', import.meta.url), 'utf8')
  const blocks: string[] = []
  for (const match of css.matchAll(/@container shui-page\b[^{]*\{/g)) {
    let depth = 1
    let end = match.index + match[0].length
    while (depth > 0 && end < css.length) {
      if (css[end] === '{') depth++
      else if (css[end] === '}') depth--
      end++
    }
    blocks.push(css.slice(match.index, end))
  }
  expect(blocks.length).toBeGreaterThan(0)
  // The frame as a rule's subject: no combinator after it.
  for (const block of blocks) expect(block).not.toMatch(/\.shui-workspace-frame(?:[.:[][^\s{,>+~]*)*\s*[{,]/)
})

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { EdgeAddZone } from './pane-controls'

describe('EdgeAddZone', () => {
  it('keeps a generous idle hover target at both responsive widths', () => {
    const markup = renderToStaticMarkup(
      <EdgeAddZone side="right" onAdd={vi.fn()} />,
    )

    expect(markup).toContain('w-3 sm:w-4')
    expect(markup).toContain('show the add-panel control (right edge)')
  })
})

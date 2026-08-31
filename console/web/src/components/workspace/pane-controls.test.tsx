import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { EdgeAddZone, ResizeHandle } from './pane-controls'

describe('EdgeAddZone', () => {
  it('keeps a generous idle hover target at both responsive widths', () => {
    const markup = renderToStaticMarkup(
      <EdgeAddZone side="right" onAdd={vi.fn()} />,
    )

    expect(markup).toContain('w-3 sm:w-4')
    expect(markup).toContain('show the add-panel control (right edge)')
    expect(markup).toContain('lucide-plus')
    expect(markup).not.toContain('edge-nudge')
    expect(markup).not.toContain('animation-delay:-5s')
    expect(markup).toContain('peer-focus-visible:outline-accent')
    expect(markup).toContain('pointer-events-none')
    expect(markup).toContain('aria-hidden="true"')
  })

  it('animates the persistent indicator only while discovery is active', () => {
    const markup = renderToStaticMarkup(
      <EdgeAddZone side="right" nudge onAdd={vi.fn()} />,
    )

    expect(markup).toContain('lucide-plus')
    expect(markup).toContain('edge-nudge')
    expect(markup).toContain('animation-delay:-5s')
  })

  it('keeps the indicator visible while temporarily disabling interaction', () => {
    const markup = renderToStaticMarkup(
      <EdgeAddZone side="left" disabled onAdd={vi.fn()} />,
    )

    expect(markup).toContain('lucide-plus')
    expect(markup).toContain('disabled=""')
    expect(markup.match(/pointer-events-none/g)).toHaveLength(2)
  })
})

describe('ResizeHandle', () => {
  it('leaves the focus order while its panel boundary collapses', () => {
    const markup = renderToStaticMarkup(
      <ResizeHandle
        value={50}
        motionState="exiting"
        onResize={vi.fn()}
        onCommit={vi.fn()}
        containerWidth={() => 1000}
      />,
    )

    expect(markup).toContain('workspace-panel-divider-exit')
    expect(markup).toContain('aria-hidden="true"')
    expect(markup).toContain('tabindex="-1"')
  })

  it('disables resize without hiding an idle boundary', () => {
    const markup = renderToStaticMarkup(
      <ResizeHandle
        value={50}
        disabled
        onResize={vi.fn()}
        onCommit={vi.fn()}
        containerWidth={() => 1000}
      />,
    )

    expect(markup).toContain('aria-disabled="true"')
    expect(markup).toContain('tabindex="-1"')
    expect(markup).toContain('pointer-events-none')
    expect(markup).not.toContain('aria-hidden')
  })
})

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import {
  EdgeAddZone,
  PanelDragHandle,
  ResizeHandle,
  SplitPreview,
} from './pane-controls'

describe('PanelDragHandle', () => {
  it('is desktop-only and exposes pointer and keyboard reorder controls', () => {
    const markup = renderToStaticMarkup(
      <PanelDragHandle
        index={1}
        count={3}
        onDragStart={vi.fn()}
        onDragEnd={vi.fn()}
        onMove={vi.fn()}
      />,
    )

    expect(markup).toContain('aria-label="reorder panel 2"')
    expect(markup).toContain(
      'aria-keyshortcuts="ArrowLeft ArrowRight Home End"',
    )
    expect(markup).toContain('draggable="true"')
    expect(markup).toContain('hidden')
    expect(markup).toContain('sm:flex')
  })
})

describe('EdgeAddZone', () => {
  it('keeps a generous idle hover target at both responsive widths', () => {
    const markup = renderToStaticMarkup(
      <EdgeAddZone side="right" onAdd={vi.fn()} />,
    )

    expect(markup).toContain('w-3 sm:w-4')
    expect(markup).toContain('show Split right control')
  })

  it('leaves a discovered edge bare: no frame, no glyph, no shake', () => {
    const markup = renderToStaticMarkup(
      <EdgeAddZone side="right" onAdd={vi.fn()} />,
    )

    expect(markup).not.toContain('lucide-plus')
    expect(markup).not.toContain('border-edge')
    expect(markup).not.toContain('edge-nudge')
    expect(markup).not.toContain('animation-delay:-5s')
    // The bare target carries its own focus treatment.
    expect(markup).toContain('focus-visible:outline-accent')
    expect(markup).not.toContain('peer-focus-visible')
  })

  it('keeps the framed, shaking sliver only while discovery is active', () => {
    const markup = renderToStaticMarkup(
      <EdgeAddZone side="right" nudge onAdd={vi.fn()} />,
    )

    expect(markup).toContain('lucide-plus')
    expect(markup).toContain('border-edge')
    expect(markup).toContain('edge-nudge')
    expect(markup).toContain('animation-delay:-5s')
    expect(markup).toContain('peer-focus-visible:outline-accent')
    expect(markup).toContain('aria-hidden="true"')
  })

  it('keeps the first-run sliver visible while temporarily disabling interaction', () => {
    const markup = renderToStaticMarkup(
      <EdgeAddZone side="left" nudge disabled onAdd={vi.fn()} />,
    )

    expect(markup).toContain('lucide-plus')
    expect(markup).toContain('disabled=""')
    expect(markup.match(/pointer-events-none/g)).toHaveLength(2)
  })

  it('disables a bare edge without drawing anything', () => {
    const markup = renderToStaticMarkup(
      <EdgeAddZone side="left" disabled onAdd={vi.fn()} />,
    )

    expect(markup).not.toContain('lucide-plus')
    expect(markup).toContain('disabled=""')
    expect(markup.match(/pointer-events-none/g)).toHaveLength(1)
  })
})

describe('SplitPreview', () => {
  it('is a floating pane-shaped card, not a dashed outline', () => {
    const markup = renderToStaticMarkup(
      <SplitPreview side="right" columns={2} onAdd={vi.fn()} />,
    )

    expect(markup).toContain('aria-label="Split right"')
    expect(markup).toContain('bg-panel-raised')
    expect(markup).toContain('shadow-floating')
    expect(markup).toContain('font-sans')
    expect(markup).toContain('split-preview-enter-right')
    expect(markup).not.toContain('border-dashed')
    expect(markup).not.toContain('backdrop-blur')
    expect(markup).not.toContain('hover:border-accent')
  })

  it('draws the columns the tab will have, the new one solid on its side', () => {
    const right = renderToStaticMarkup(
      <SplitPreview side="right" columns={2} onAdd={vi.fn()} />,
    )
    expect(right.match(/bg-ink\/20/g)).toHaveLength(2)
    expect(right.match(/split-preview-column-right/g)).toHaveLength(1)
    // The solid column is the last rectangle on the right edge…
    expect(right.indexOf('bg-ink/20')).toBeLessThan(
      right.indexOf('split-preview-column-right'),
    )

    const left = renderToStaticMarkup(
      <SplitPreview side="left" columns={1} onAdd={vi.fn()} />,
    )
    expect(left.match(/bg-ink\/20/g)).toHaveLength(1)
    // …and the first one on the left edge.
    expect(left.indexOf('split-preview-column-left')).toBeLessThan(
      left.indexOf('bg-ink/20'),
    )
  })

  it('caps the schematic so a wide tab still fits the card', () => {
    const markup = renderToStaticMarkup(
      <SplitPreview side="right" columns={12} onAdd={vi.fn()} />,
    )

    expect(markup.match(/bg-ink\/20/g)).toHaveLength(3)
  })

  it('teaches each edge its own key: the bracket on its side', () => {
    const right = renderToStaticMarkup(
      <SplitPreview side="right" columns={1} onAdd={vi.fn()} />,
    )
    expect(right).toContain('<kbd')
    expect(right).toContain('title="Split right (alt ])"')

    const left = renderToStaticMarkup(
      <SplitPreview side="left" columns={1} onAdd={vi.fn()} />,
    )
    expect(left).toContain('<kbd')
    expect(left).toContain('title="Split left (alt [)"')
  })

  it('mentions the other edge only during discovery', () => {
    const nudged = renderToStaticMarkup(
      <SplitPreview side="right" columns={1} nudge onAdd={vi.fn()} />,
    )
    expect(nudged).toContain('The left edge works too.')

    const settled = renderToStaticMarkup(
      <SplitPreview side="right" columns={1} onAdd={vi.fn()} />,
    )
    expect(settled).not.toContain('edge works too')
  })

  it('fades out along its edge while closing', () => {
    const markup = renderToStaticMarkup(
      <SplitPreview side="left" columns={1} closing onAdd={vi.fn()} />,
    )

    expect(markup).toContain('split-preview-exit-left')
    expect(markup).not.toContain('split-preview-enter-left')
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

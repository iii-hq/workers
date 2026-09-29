// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { normalizeMermaidSvg, renderMermaid } from './mermaid'

const { initialize, render } = vi.hoisted(() => ({
  initialize: vi.fn(),
  render: vi.fn(),
}))
vi.mock('mermaid', () => ({ default: { initialize, render } }))

const SVG = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 60" />'

beforeEach(() => {
  // jsdom has no SVG layout; browser coverage below exercises the real API.
  Object.defineProperty(SVGElement.prototype, 'getBBox', {
    configurable: true,
    value: vi.fn(() => ({ x: 0, y: 0, width: 0, height: 0 })),
  })
  initialize.mockReset()
  render.mockReset().mockImplementation(async (_id, _source, host: Element) => {
    host.append(document.createElement('span'))
    return { svg: SVG }
  })
})

afterEach(() => {
  delete (SVGElement.prototype as SVGElement & { getBBox?: unknown }).getBBox
})

describe('Mermaid rendering boundary', () => {
  it('locks down security and size settings and removes the temporary host', async () => {
    const children = document.body.childElementCount
    expect(await renderMermaid('flowchart LR\nA --> B', 'light')).toBe(
      normalizeMermaidSvg(SVG),
    )
    expect(initialize).toHaveBeenCalledWith(
      expect.objectContaining({
        securityLevel: 'strict',
        startOnLoad: false,
        htmlLabels: false,
        suppressErrorRendering: true,
        maxTextSize: 50_000,
        maxEdges: 500,
        secure: expect.arrayContaining([
          'secure',
          'securityLevel',
          'maxEdges',
          'maxTextSize',
        ]),
      }),
    )
    expect(document.body.childElementCount).toBe(children)
  })

  it('serializes diagrams so a second theme cannot change an in-flight render', async () => {
    let finish!: (value: { svg: string }) => void
    let started!: () => void
    const ready = new Promise<void>((resolve) => {
      started = resolve
    })
    render.mockImplementationOnce(() => {
      started()
      return new Promise((resolve) => {
        finish = resolve
      })
    })
    const first = renderMermaid('first', 'light')
    const second = renderMermaid('second', 'dark')
    await ready
    expect(initialize).toHaveBeenCalledTimes(1)
    finish({ svg: SVG })
    await Promise.all([first, second])
    expect(initialize.mock.calls.map(([config]) => config.theme)).toEqual([
      'default',
      'dark',
    ])
    expect(render.mock.calls[0][0]).not.toBe(render.mock.calls[1][0])
  })

  it('cleans up failures without blocking later diagrams', async () => {
    const children = document.body.childElementCount
    render.mockRejectedValueOnce(new Error('parse failed'))
    await expect(renderMermaid('broken', 'light')).rejects.toThrow(
      'parse failed',
    )
    expect(document.body.childElementCount).toBe(children)
    await expect(renderMermaid('valid', 'light')).resolves.toBe(
      normalizeMermaidSvg(SVG),
    )
  })

  it('rejects oversized input before invoking Mermaid', async () => {
    await expect(renderMermaid('x'.repeat(50_001), 'light')).rejects.toThrow(
      'size limit',
    )
    expect(initialize).not.toHaveBeenCalled()
    expect(render).not.toHaveBeenCalled()
  })
})

describe('standalone Mermaid SVG dimensions', () => {
  it('exports intrinsic dimensions and a padded viewport for a wide diagram', () => {
    const result = normalizeMermaidSvg(
      '<svg xmlns="http://www.w3.org/2000/svg" width="100%" viewBox="0 0 837.1 63" style="max-width: 837.1px; height: 100%; background: white"><g /></svg>',
    )
    const svg = new DOMParser().parseFromString(
      result,
      'image/svg+xml',
    ).documentElement
    expect(svg.getAttribute('width')).toBe('869.1')
    expect(svg.getAttribute('height')).toBe('95')
    expect(svg.getAttribute('viewBox')).toBe('-16 -16 869.1 95')
    expect(svg.getAttribute('preserveAspectRatio')).toBe('xMidYMid meet')
    expect(svg.getAttribute('style')).not.toContain('max-width')
    expect(svg.getAttribute('style')).not.toContain('height')
    expect(svg.getAttribute('style')).toContain('background')
    expect(svg.querySelector('g')).not.toBeNull()
  })

  it('keeps negative origins and tall or fractional diagrams intact', () => {
    const svg = new DOMParser().parseFromString(
      normalizeMermaidSvg(
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="-40,-20,100.5,1200.25" />',
      ),
      'image/svg+xml',
    ).documentElement
    expect(svg.getAttribute('viewBox')).toBe('-56 -36 132.5 1232.25')
    expect(svg.getAttribute('width')).toBe('132.5')
    expect(svg.getAttribute('height')).toBe('1232.25')
  })

  it.each(['', '0 0 0 20', '0 0 20 -1', '0 0 NaN 20'])(
    'rejects an invalid viewport instead of silently clipping it: %s',
    (viewBox) => {
      expect(() => normalizeMermaidSvg(`<svg viewBox="${viewBox}" />`)).toThrow(
        'viewport',
      )
    },
  )

  it('expands a stale viewport to include bottom and right-hand nodes', () => {
    vi.mocked(
      (SVGElement.prototype as SVGGraphicsElement).getBBox,
    ).mockReturnValue({
      x: 8,
      y: 8,
      width: 1020,
      height: 310,
    } as DOMRect)
    const xml = normalizeMermaidSvg(
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 868 218"><g /></svg>',
    )
    const svg = new DOMParser().parseFromString(
      xml,
      'image/svg+xml',
    ).documentElement
    expect(svg.getAttribute('viewBox')).toBe('-16 -16 1060 350')
    expect(svg.getAttribute('width')).toBe('1060')
    expect(svg.getAttribute('height')).toBe('350')
  })

  it('includes content at negative coordinates without shrinking reserved margins', () => {
    vi.mocked(
      (SVGElement.prototype as SVGGraphicsElement).getBBox,
    ).mockReturnValue({
      x: -60,
      y: -40,
      width: 100,
      height: 90,
    } as DOMRect)
    const xml = normalizeMermaidSvg(
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 600" />',
    )
    expect(xml).toContain('viewBox="-76 -56 892 672"')
  })

  it('measures in an attached shadow root and always removes the measurement host', () => {
    const children = document.body.childElementCount
    vi.mocked(
      (SVGElement.prototype as SVGGraphicsElement).getBBox,
    ).mockImplementation(function (this: SVGGraphicsElement) {
      expect(this.isConnected).toBe(true)
      expect(this.getRootNode()).toBeInstanceOf(ShadowRoot)
      throw new Error('measurement failed')
    })
    expect(() => normalizeMermaidSvg(SVG)).toThrow('measurement failed')
    expect(document.body.childElementCount).toBe(children)
  })
})

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { AutocompactLine, ChipAutoLabel, HeroAutoLabel, StackTick } from './autocompact'
import { RingGauge } from './icons'

const mark = { effective: 160_000, rule: { scope: 'project' as const, project_name: 'acme-web' } }
const atLimit = { effective: null, rule: { scope: 'global' as const } }

describe('the ring tick', () => {
  it('draws an amber mark across the track at the ratio', () => {
    const html = renderToStaticMarkup(<RingGauge ratio={0.5} color="red" tick={0.25} />)
    expect(html).toContain('data-ring-tick')
    expect(html).toContain('stroke="var(--color-warn)"')
    // A quarter turn from twelve o'clock is three o'clock: the mark runs
    // along y = centre (8) out to the right edge (16).
    expect(html).toMatch(/y1="8"/)
    expect(html).toMatch(/x2="16"/)
  })

  it('draws none without a tick', () => {
    expect(renderToStaticMarkup(<RingGauge ratio={0.5} color="red" />)).not.toContain('data-ring-tick')
    expect(renderToStaticMarkup(<RingGauge ratio={0.5} color="red" tick={null} />)).not.toContain('data-ring-tick')
  })
})

describe('the chip label', () => {
  it('reads "· auto 160k" after the counts', () => {
    const html = renderToStaticMarkup(<ChipAutoLabel total={90_000} mark={mark} running={false} />)
    expect(html).toContain('harness-ui-chip-auto')
    expect(html).toContain('auto 160k')
    expect(html).toContain('·')
    expect(html).not.toContain('data-after-turn')
  })

  it('reads "Compacts after this turn" while a running turn is past the size', () => {
    const html = renderToStaticMarkup(<ChipAutoLabel total={171_000} mark={mark} running />)
    expect(html).toContain('Compacts after this turn')
    expect(html).toContain('data-after-turn=""')
  })

  it('is absent at the model limit and from an older harness', () => {
    expect(renderToStaticMarkup(<ChipAutoLabel total={171_000} mark={atLimit} running />)).toBe('')
    expect(renderToStaticMarkup(<ChipAutoLabel total={171_000} mark={null} running />)).toBe('')
  })
})

describe('the breakdown', () => {
  it('labels the hero as the chip does', () => {
    expect(renderToStaticMarkup(<HeroAutoLabel total={90_000} mark={mark} />)).toContain('auto 160k')
    expect(renderToStaticMarkup(<HeroAutoLabel total={171_000} mark={mark} running />)).toContain(
      'Compacts after this turn',
    )
    expect(renderToStaticMarkup(<HeroAutoLabel total={90_000} mark={atLimit} />)).toBe('')
  })

  it('marks the bar at the ratio', () => {
    const html = renderToStaticMarkup(<StackTick tick={0.8} />)
    expect(html).toContain('data-autocompact-tick')
    expect(html).toContain('left:80%')
  })

  it('writes the line, or the model limit, and nothing from an older harness', () => {
    expect(renderToStaticMarkup(<AutocompactLine mark={mark} />)).toContain('Auto-compact at 160k · acme-web rule')
    expect(renderToStaticMarkup(<AutocompactLine mark={atLimit} />)).toContain('At the model&#x27;s limit')
    expect(renderToStaticMarkup(<AutocompactLine mark={null} />)).toBe('')
  })
})

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { effortRatio, ReasoningEffortSlider } from './ReasoningEffortSlider'

const OPTIONS = [
  { effort: 'default', description: 'use the model default' },
  { effort: 'low', description: 'quick reasoning' },
  { effort: 'medium', description: 'balanced' },
  { effort: 'high', description: 'deeper reasoning' },
  { effort: 'xhigh' },
]

describe('effortRatio', () => {
  it('spreads levels evenly from 0 to 1', () => {
    expect(effortRatio(0, 5)).toBe(0)
    expect(effortRatio(2, 5)).toBe(0.5)
    expect(effortRatio(4, 5)).toBe(1)
    expect(effortRatio(0, 1)).toBe(0)
    expect(effortRatio(9, 5)).toBe(1)
  })
})

describe('ReasoningEffortSlider', () => {
  it('drives the colour ramp from the selected level', () => {
    const html = renderToStaticMarkup(
      <ReasoningEffortSlider
        options={OPTIONS}
        value="high"
        onChange={() => {}}
      />,
    )
    expect(html).toContain('--effort-ratio:0.75')
    expect(html).toContain('type="range"')
    expect(html).toContain('value="3"')
    expect(html).toContain('aria-valuetext="high"')
    // The description moved into the tooltip; no copy line under the track.
    expect(html).toContain('title="deeper reasoning"')
    expect(html).not.toContain('>deeper reasoning<')
    expect(html.match(/class="reasoning-effort__dot"/g)).toHaveLength(5)
    expect(html).toContain('--effort-stop:0.25')
  })

  it('shows the level name through the text swap slot', () => {
    const html = renderToStaticMarkup(
      <ReasoningEffortSlider
        options={OPTIONS}
        value="medium"
        onChange={() => {}}
      />,
    )
    expect(html).toMatch(
      /<span[^>]*data-effort-label[^>]*class="t-text-swap[^"]*"[^>]*>medium<\/span>/,
    )
    // Every level is stacked invisibly so the cell keeps the widest width.
    expect(
      html.match(/class="invisible col-start-1 row-start-1 text-right"/g),
    ).toHaveLength(5)
  })

  it('falls back to the first level when nothing matches', () => {
    const html = renderToStaticMarkup(
      <ReasoningEffortSlider
        options={OPTIONS}
        value="ultra"
        onChange={() => {}}
      />,
    )
    expect(html).toContain('--effort-ratio:0')
    expect(html).toContain('aria-valuetext="default"')
  })
})

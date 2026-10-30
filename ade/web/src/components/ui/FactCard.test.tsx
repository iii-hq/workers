import { Zap } from 'lucide-react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import {
  Fact,
  FactCard,
  FactCardHeader,
  FactCardStatus,
  FactList,
  FactMeter,
} from './FactCard'

describe('FactCard', () => {
  it('is a popover-styled Inter 14px shell carrying its accent as a variable', () => {
    const out = renderToStaticMarkup(
      <FactCard accent="var(--color-worker-3)">x</FactCard>,
    )
    expect(out).toContain('data-slot="fact-card"')
    expect(out).toContain('font-inter')
    expect(out).toContain('text-sm')
    expect(out).toContain('bg-popover')
    expect(out).toContain('--fact-accent:var(--color-worker-3)')
  })

  it('tints the icon chip from the accent and sets machine names in mono', () => {
    const out = renderToStaticMarkup(
      <FactCardHeader icon={Zap} title="harness::turn step" mono />,
    )
    expect(out).toContain('var(--fact-accent,var(--color-ink))')
    expect(out).toMatch(/font-mono[^"]*">harness::turn step</)
  })

  it('writes the status in the semantic tone, never the accent', () => {
    const out = renderToStaticMarkup(
      <FactCardStatus tone="alert" label="error" meta="harness" />,
    )
    expect(out).toContain('bg-alert')
    expect(out).toContain('text-alert-strong')
    expect(out).toContain('harness')
    expect(out).not.toContain('fact-accent')
  })

  it('renders label/value rows as a definition list with mono tabular values', () => {
    const out = renderToStaticMarkup(
      <FactList>
        <Fact
          label="Duration"
          mono
          align="end"
          data-technical-detail="duration"
        >
          10.60s
        </Fact>
        <Fact label="Worker" muted>
          harness
        </Fact>
      </FactList>,
    )
    expect(out).toMatch(/<dt[^>]*text-ink-faint[^>]*>Duration<\/dt>/)
    expect(out).toMatch(
      /<dd[^>]*font-mono[^>]*tabular-nums[^>]*text-right[^>]*data-technical-detail="duration"[^>]*>10\.60s<\/dd>/,
    )
    expect(out).toMatch(/<dd[^>]*text-ink-faint[^>]*>harness<\/dd>/)
  })

  it('clamps the meter and exposes it as a progressbar', () => {
    const out = renderToStaticMarkup(
      <FactMeter value={140} label="share of the trace" />,
    )
    expect(out).toContain('role="progressbar"')
    expect(out).toContain('aria-valuenow="100"')
    expect(out).toContain('width:100%')
  })
})

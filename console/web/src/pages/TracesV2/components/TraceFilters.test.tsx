import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { StatsBlock } from './TraceFilters'

describe('StatsBlock', () => {
  it('labels page-scoped error and duration metrics honestly', () => {
    const html = renderToStaticMarkup(
      <StatsBlock
        stats={{
          totalTraces: 2_601,
          pageTraceCount: 50,
          errorCount: 0,
          avgDuration: 12,
        }}
      />,
    )

    expect(html).toContain('2601')
    expect(html).toContain('50 traces on this page, 2601 total matching traces')
    expect(html).toContain('Page errors')
    expect(html).toContain('Page avg')
  })
})

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
    expect(html).not.toContain('data-trace-list-held')
  })

  it('shows the held-new segment as a quiet count, or "latest" from a later page', () => {
    const stats = {
      totalTraces: 10,
      pageTraceCount: 10,
      errorCount: 0,
      avgDuration: 1,
    }
    const onPage1 = renderToStaticMarkup(
      <StatsBlock
        stats={stats}
        heldNew={{ count: 3, onPage1: true, onShow: () => {} }}
      />,
    )
    expect(onPage1).toContain('data-trace-list-held="3"')
    expect(onPage1).toContain('show 3 new traces')
    expect(onPage1).toContain('>new<')

    const later = renderToStaticMarkup(
      <StatsBlock
        stats={stats}
        heldNew={{ count: 3, onPage1: false, onShow: () => {} }}
      />,
    )
    expect(later).toContain('show the latest traces on page 1')
    expect(later).toContain('>latest<')

    expect(
      renderToStaticMarkup(
        <StatsBlock
          stats={stats}
          heldNew={{ count: 0, onPage1: true, onShow: () => {} }}
        />,
      ),
    ).not.toContain('data-trace-list-held')
  })
})

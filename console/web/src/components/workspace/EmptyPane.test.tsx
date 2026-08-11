import { Blocks } from 'lucide-react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { EmptyPane } from './EmptyPane'
import { filterScreenOptions } from './empty-pane-search'
import type { ScreenOption } from './use-screen-options'

const pages = Array.from({ length: 50 }, (_, index): ScreenOption => {
  const number = index + 1
  return {
    value: `ext:page-${number}`,
    label: `page ${number}`,
    description: `Worker page ${number}.`,
    keywords: [`worker-${number}`],
    icon: Blocks,
  }
})

describe('filterScreenOptions', () => {
  it('keeps all pages available when the query is blank', () => {
    expect(filterScreenOptions(pages, '')).toEqual(pages)
  })

  it('finds pages by loose label abbreviations and metadata', () => {
    expect(
      filterScreenOptions(pages, 'pge 49').map((page) => page.label),
    ).toEqual(['page 49'])
    expect(
      filterScreenOptions(pages, 'worker-42').map((page) => page.label),
    ).toEqual(['page 42'])
  })

  it('returns an empty list when no page matches', () => {
    expect(filterScreenOptions(pages, 'unavailable')).toEqual([])
  })
})

describe('EmptyPane', () => {
  it('renders a scrollable option for every available page', () => {
    const markup = renderToStaticMarkup(
      <EmptyPane screenOptions={pages} onAttach={vi.fn()} onRemove={vi.fn()} />,
    )

    expect(markup.match(/role="option"/g)).toHaveLength(50)
    expect(markup).toContain('>50</div>')
    expect(markup).toContain('aria-label="remove this panel"')
    expect(markup).toContain('max-h-72 overflow-y-auto')
  })
})

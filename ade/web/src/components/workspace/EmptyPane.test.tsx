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
  it('renders an option for every available page in a scrolling pane', () => {
    const markup = renderToStaticMarkup(
      <EmptyPane screenOptions={pages} onAttach={vi.fn()} onRemove={vi.fn()} />,
    )

    expect(markup.match(/role="option"/g)).toHaveLength(50)
    expect(markup).toContain('aria-label="remove this panel"')
    expect(markup).toContain('overflow-y-auto')
    expect(markup).not.toContain('max-h-72')
    expect(markup).not.toContain('autofocus')
  })

  it('groups console pages ahead of worker pages, each with its count', () => {
    const markup = renderToStaticMarkup(
      <EmptyPane
        screenOptions={[
          {
            value: 'chat',
            label: 'chat',
            description: 'Conversations and tools.',
            icon: Blocks,
          },
          ...pages,
        ]}
        onAttach={vi.fn()}
      />,
    )

    const consoleAt = markup.indexOf('Console<')
    const workersAt = markup.indexOf('From workers<')
    expect(consoleAt).toBeGreaterThan(-1)
    expect(workersAt).toBeGreaterThan(consoleAt)
    expect(markup).toContain('>1</span>')
    expect(markup).toContain('>50</span>')
  })

  it('draws no highlight until the launcher is searched or navigated', () => {
    const markup = renderToStaticMarkup(
      <EmptyPane screenOptions={pages} onAttach={vi.fn()} />,
    )

    expect(markup).not.toContain('aria-selected="true"')
    expect(markup).not.toContain('aria-activedescendant')
  })
})

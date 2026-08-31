import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { PageSidebar } from './PageSidebar'
import { TooltipProvider } from './Tooltip'

function renderSidebar(sidebar: React.ReactNode): string {
  return renderToStaticMarkup(
    <TooltipProvider delayDuration={0}>{sidebar}</TooltipProvider>,
  )
}

describe('PageSidebar', () => {
  it('preserves the fixed-width primitive for existing consumers', () => {
    const html = renderSidebar(
      <PageSidebar width={312} className="catalog-navigation">
        navigation
      </PageSidebar>,
    )

    expect(html).toContain('style="width:312px"')
    expect(html).toContain('catalog-navigation')
    expect(html).toContain('>navigation</aside>')
    expect(html).not.toContain('aria-expanded')
  })

  it('renders one stable managed aside with accessible controls', () => {
    const html = renderSidebar(
      <PageSidebar
        label="projects"
        defaultWidth={240}
        minWidth={180}
        maxWidth={360}
        collapsible
        resizable
        header={<button type="button">new project</button>}
        collapsedActions={<button type="button">new</button>}
      >
        <input aria-label="filter projects" />
      </PageSidebar>,
    )

    expect(html.match(/<aside/g)).toHaveLength(1)
    expect(html).toContain('aria-label="projects"')
    expect(html).toContain('style="width:240px"')
    expect(html).toContain('aria-label="collapse projects"')
    expect(html).toContain('aria-expanded="true"')
    expect(html).toContain('aria-label="resize projects"')
    expect(html).toContain('aria-valuemin="180"')
    expect(html).toContain('aria-valuemax="360"')
  })

  it('keeps expanded content mounted and inert while collapsed', () => {
    const html = renderSidebar(
      <PageSidebar label="sessions" collapsible defaultCollapsed>
        <button type="button">session one</button>
      </PageSidebar>,
    )

    expect(html).toContain('data-collapsed=""')
    expect(html).toContain('style="width:36px"')
    expect(html).toContain('aria-label="expand sessions"')
    expect(html).toContain('aria-expanded="false"')
    expect(html).toContain('session one')
    expect(html).toContain('inert=""')
  })

  it('narrows a host-owned sidebar to a closed drawer rail', () => {
    const html = renderSidebar(
      <PageSidebar label="files" collapsible narrow>
        file tree
      </PageSidebar>,
    )

    expect(html).toContain('style="width:36px"')
    expect(html).toContain('data-narrow=""')
    expect(html).toContain('data-drawer="closed"')
    expect(html).toContain('data-collapsed=""')
    expect(html).toContain('file tree')
    expect(html).toContain('aria-label="expand files"')
    expect(html).not.toContain('aria-label="close files"')
  })

  it('keeps the full-width presentation for a page that controls collapse', () => {
    const html = renderSidebar(
      <PageSidebar label="files" collapsible collapsed={false} narrow>
        file tree
      </PageSidebar>,
    )

    expect(html).toContain('style="width:100%"')
    expect(html).toContain('file tree')
    expect(html).not.toContain('data-collapsed')
    expect(html).not.toContain('data-drawer')
    expect(html).not.toContain('aria-label="expand files"')
  })
})

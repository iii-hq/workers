import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
} from './CollapsibleCard'

function renderCard(open?: boolean) {
  return renderToStaticMarkup(
    createElement(
      CollapsibleCard,
      open === undefined ? {} : { open },
      createElement(CollapsibleCardTrigger, null, 'Activity'),
      createElement(
        CollapsibleCardContent,
        null,
        createElement('button', { type: 'button' }, 'Preserved action'),
      ),
    ),
  )
}

describe('CollapsibleCard', () => {
  it('renders a closed, accessible disclosure while preserving its content', () => {
    const html = renderCard(false)
    const controls = html.match(/aria-controls="([^"]+)"/)?.[1]
    const contentId = html.match(/<section id="([^"]+)"/)?.[1]

    expect(html).toContain('iii-ui-card')
    expect(html).toContain('iii-ui-collapsible-card')
    expect(html).toContain('aria-expanded="false"')
    expect(html).toContain('aria-hidden="true"')
    expect(html).toContain('aria-labelledby=')
    expect(html).toContain('data-state="closed"')
    expect(html).toContain('inert=""')
    expect(html).toContain('Preserved action')
    expect(controls).toBe(contentId)
  })

  it('exposes the controlled open state to the trigger and animated region', () => {
    const html = renderCard(true)

    expect(html).toContain('aria-expanded="true"')
    expect(html).toContain('aria-hidden="false"')
    expect(html).toContain('data-state="open"')
    expect(html).not.toContain('inert=""')
  })

  it('supports an initially open uncontrolled card', () => {
    const html = renderToStaticMarkup(
      <CollapsibleCard defaultOpen>
        <CollapsibleCardTrigger>Activity</CollapsibleCardTrigger>
        <CollapsibleCardContent>Details</CollapsibleCardContent>
      </CollapsibleCard>,
    )

    expect(html).toContain('aria-expanded="true"')
    expect(html).toContain('data-state="open"')
  })
})

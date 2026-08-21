import { Activity } from 'lucide-react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Badge } from './Badge'

describe('Badge', () => {
  it.each([
    ['default', 'bg-surface', 'text-ink-faint'],
    ['ok', 'bg-ok-muted', 'text-ok'],
    ['accent', 'bg-accent-muted', 'text-accent'],
    ['warn', 'bg-warn-muted', 'text-warn'],
    ['alert', 'bg-alert-muted', 'text-alert'],
  ] as const)('renders the %s semantic treatment', (variant, fill, ink) => {
    const html = renderToStaticMarkup(
      <Badge variant={variant}>{variant}</Badge>,
    )

    expect(html).toContain(`data-badge-variant="${variant}"`)
    expect(html).toContain(fill)
    expect(html).toContain(ink)
  })

  it('owns the shared ActivityStatus pill shape and responsive type scale', () => {
    const html = renderToStaticMarkup(
      <Badge variant="ok" className="tabular-nums" title="healthy">
        <Activity aria-hidden className="size-4" />
        Healthy
      </Badge>,
    )

    expect(html).toContain('rounded-full')
    expect(html).toContain('px-2.5')
    expect(html).toContain('py-1')
    expect(html).toContain('text-base')
    expect(html).toContain('sm:text-xs')
    expect(html).toContain('tabular-nums')
    expect(html).toContain('title="healthy"')
  })
})

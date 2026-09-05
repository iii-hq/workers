import { Activity } from 'lucide-react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ActivityStatus } from './ActivityStatus'

describe('ActivityStatus', () => {
  it.each([
    ['positive', 'ok', 'bg-ok-muted', 'stroke-ok'],
    ['neutral', 'default', 'bg-surface', 'stroke-trigger-running'],
    ['accent', 'accent', 'bg-accent-muted', 'stroke-accent'],
    ['warning', 'warn', 'bg-warn-muted', 'stroke-warn'],
    ['danger', 'alert', 'bg-alert-muted', 'stroke-alert'],
  ] as const)(
    'applies the %s tone to the badge and icon only',
    (tone, variant, badge, icon) => {
      const html = renderToStaticMarkup(
        <ActivityStatus
          label="Active"
          detail="Active for 5m"
          icon={Activity}
          tone={tone}
        />,
      )

      expect(html).toContain(`data-activity-status-tone="${tone}"`)
      expect(html).toContain(`data-badge-variant="${variant}"`)
      expect(html).toContain(badge)
      expect(html).toContain(icon)
      expect(html).toContain('role="status"')
      expect(html).toContain('Active for 5m')
      // The badge is the one status flag; the detail line carries no dot.
      expect(html).not.toContain('size-2 shrink-0 rounded-full')
    },
  )

  it('merges call-site classes and exposes motion without owning spacing', () => {
    const html = renderToStaticMarkup(
      <ActivityStatus
        className="justify-end"
        label="Working"
        icon={Activity}
        motion="spin"
      />,
    )

    expect(html).toContain('justify-end')
    expect(html).toContain('animate-spin')
    const rootClasses = html.match(/^<div class="([^"]+)"/)?.[1]
    expect(rootClasses).not.toContain('mt-')
  })
})

import { Activity } from 'lucide-react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ActivityStatus } from './ActivityStatus'

describe('ActivityStatus', () => {
  it.each([
    ['positive', 'ok', 'bg-ok-muted', 'stroke-ok', 'bg-ok'],
    [
      'neutral',
      'default',
      'bg-surface',
      'stroke-trigger-running',
      'bg-trigger-running',
    ],
    ['accent', 'accent', 'bg-accent-muted', 'stroke-accent', 'bg-accent'],
    ['warning', 'warn', 'bg-warn-muted', 'stroke-warn', 'bg-warn'],
    ['danger', 'alert', 'bg-alert-muted', 'stroke-alert', 'bg-alert'],
  ] as const)(
    'applies the %s tone to the badge, icon, and activity dot',
    (tone, variant, badge, icon, dot) => {
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
      expect(html).toContain(dot)
      expect(html).toContain('role="status"')
      expect(html).toContain('Active for 5m')
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

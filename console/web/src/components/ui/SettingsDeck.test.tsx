import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { SettingsDeck } from './SettingsDeck'

describe('SettingsDeck', () => {
  it('shows only the overview at the root level', () => {
    const html = renderToStaticMarkup(
      <SettingsDeck
        open={false}
        title="Primary"
        overview={<button type="button">Open primary</button>}
        detail={<input aria-label="Connection URL" />}
        onBack={() => {}}
      />,
    )

    expect(html).toContain('data-state="overview"')
    expect(html).toContain('data-settings-deck-pane="overview"')
    expect(html).toMatch(/hidden=""[^>]+data-settings-deck-pane="detail"/)
    expect(html).toContain('Open primary')
  })

  it('exposes a labelled detail level with a consistent back action', () => {
    const html = renderToStaticMarkup(
      <SettingsDeck
        open
        title="Primary"
        description="Connection settings"
        backLabel="Connections"
        backAriaLabel="Back to database connections"
        overview={<button type="button">Open primary</button>}
        detail={<input aria-label="Connection URL" />}
        onBack={() => {}}
      />,
    )

    expect(html).toContain('data-state="detail"')
    expect(html).toContain('@container')
    expect(html).toContain('@lg:min-h-8')
    expect(html).toMatch(/hidden=""[^>]+data-settings-deck-pane="overview"/)
    expect(html).toMatch(/<section[^>]+aria-labelledby="[^"]+"/)
    expect(html).toContain('aria-label="Back to database connections"')
    expect(html).toContain('Connection settings')
    expect(html).toContain('Connection URL')
  })
})

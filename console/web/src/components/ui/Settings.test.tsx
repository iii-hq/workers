import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { SettingsList, SettingsRow, SettingsSection } from './Settings'

describe('settings primitives', () => {
  it('labels the section and exposes list semantics', () => {
    const html = renderToStaticMarkup(
      <SettingsSection
        title="Notifications"
        description="Choose which events should reach you."
        action={<button type="button">Reset</button>}
      >
        <SettingsList>
          <SettingsRow
            label="Critical requests"
            description="Notify when a decision needs attention."
            meta="Desktop and mobile"
            layout="inline"
            control={<input aria-label="Critical requests" type="checkbox" />}
          />
        </SettingsList>
      </SettingsSection>,
    )

    expect(html).toMatch(/<section[^>]+aria-labelledby="([^"]+)"/)
    expect(html).toMatch(/<section[^>]+aria-describedby="([^"]+)"/)
    expect(html).toContain('role="list"')
    expect(html).toContain('role="listitem"')
    expect(html).toContain('data-layout="inline"')
    expect(html).toContain('Critical requests')
    expect(html).toContain('Desktop and mobile')
    expect(html).toContain('>Reset</button>')
  })

  it('composes a value control and a secondary action in the trailing slot', () => {
    const html = renderToStaticMarkup(
      <SettingsList aria-label="Account settings">
        <SettingsRow
          label="Device ID"
          control={<span>device-123</span>}
          action={<button type="button">Copy</button>}
        />
        <SettingsRow label="No trailing value" layout="stacked" />
      </SettingsList>,
    )

    expect(html).toContain('aria-label="Account settings"')
    expect(html).toContain('iii-ui-settings-row__control')
    expect(html).toContain('device-123')
    expect(html).toContain('>Copy</button>')
    expect(html).toContain('data-layout="stacked"')
    expect(html).toContain('data-has-trailing="true"')
  })
})

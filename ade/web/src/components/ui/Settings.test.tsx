import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Input } from './Input'
import {
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
} from './Settings'
import { Switch } from './Switch'

/** Attribute text as React serialises it, back to the class names. */
const decode = (text: string) =>
  text.replaceAll('&gt;', '>').replaceAll('&lt;', '<').replaceAll('&amp;', '&')

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

  it('associates a reusable field label, guidance, error, and deep-link path', () => {
    const html = renderToStaticMarkup(
      <SettingsList>
        <SettingsField
          id="pool-size"
          field="databases.primary.pool.max"
          label="Maximum connections"
          description="Upper bound for open connections."
          error="Must be at least one."
          controlSize="compact"
          renderControl={(controlProps) => (
            <Input value="0" onChange={() => {}} {...controlProps} />
          )}
        />
      </SettingsList>,
    )

    expect(html).toMatch(
      /<label [^>]*for="pool-size"[^>]*>Maximum connections<\/label>/,
    )
    expect(html).toContain('name="databases.primary.pool.max"')
    expect(html).toContain('data-field="databases.primary.pool.max"')
    expect(html).toContain('data-settings-field-control="true"')
    expect(html).toContain('aria-invalid="true"')
    expect(html).toMatch(/aria-describedby="[^"]+-description [^"]+-error"/)
    expect(html).toContain('role="alert"')
  })

  it('supports intrinsic-width controls such as switches', () => {
    const html = renderToStaticMarkup(
      <SettingsField
        id="notifications"
        field="notifications.enabled"
        label="Notifications"
        layout="inline"
        controlSize="fit"
        renderControl={(controlProps) => (
          <input {...controlProps} type="checkbox" />
        )}
      />,
    )

    expect(html).toContain('@[30rem]:w-auto')
    expect(html).toContain('data-layout="inline"')
  })

  // QA 2026-11-16 F9: Judge's "Keep every local provider loaded" took the
  // default 320px control width and drew a long pill with the knob at its
  // left end. A switch keeps its own size whatever `controlSize` is.
  it('never stretches a switch to the field width', () => {
    const html = renderToStaticMarkup(
      <SettingsField
        id="preload"
        label="Keep every local provider loaded"
        renderControl={(controlProps) => (
          <Switch {...controlProps} checked={false} onChange={() => {}} />
        )}
      />,
    )
    const wrapper = html.match(
      /<div data-settings-field-control="true" class="([^"]+)"/,
    )?.[1]
    expect(wrapper).toBeDefined()
    const classes = decode(wrapper ?? '').split(' ')
    // Stretched only when it is not a switch, and the slot fits it.
    expect(classes).not.toContain('[&>*]:w-full')
    expect(
      classes.some(
        (name) => name.includes(':not(') && name.includes('[role=switch]'),
      ),
    ).toBe(true)
    expect(
      classes.some(
        (name) => name.startsWith('has-[>') && name.endsWith(':w-auto'),
      ),
    ).toBe(true)
    expect(html).toContain('role="switch"')
  })

  // QA 2026-11-16 F10: worker forms built on SettingsField read at 14/12px
  // beside General's 16/14px rows: the field set its own label and
  // description sizes instead of taking the row recipe's.
  it('lets the row recipe set the label and description size', () => {
    const html = renderToStaticMarkup(
      <SettingsField
        id="prefix"
        label="Ticket key prefix"
        description="Prefix for the human-readable key."
        renderControl={(controlProps) => (
          <Input value="KAN" onChange={() => {}} {...controlProps} />
        )}
      />,
    )
    const label = html.match(/<label [^>]*class="([^"]+)"/)?.[1] ?? ''
    const description =
      html.match(/data-slot="field-description"[^>]*class="([^"]+)"/)?.[1] ??
      html.match(/class="([^"]+)"[^>]*data-slot="field-description"/)?.[1] ??
      ''
    for (const classes of [label, description]) {
      expect(classes.split(' ')).toContain('text-[length:inherit]')
      expect(classes).not.toMatch(/(^| )text-(xs|sm)( |$)/)
    }
  })
})

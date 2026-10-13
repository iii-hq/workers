// @vitest-environment jsdom
import type { ConfigFormProps, Host, RawValueInputProps, SettingsFieldProps, StatusPanelProps } from '@iii-dev/console-ui'
import { act, type ComponentProps, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WhatsAppConfigForm } from '../src/configuration'
import { CLOUD_DEFAULTS, DEFAULTS, type ConfigStatus } from '../src/model'

vi.mock('@iii-dev/console-ui', () => ({
  Badge: ({ children }: { children: ReactNode }) => <span>{children}</span>,
  Button: ({ children, variant: _variant, ...props }: ComponentProps<'button'> & { variant?: string }) => <button {...props}>{children}</button>,
  Input: ({ onChange, ...props }: Omit<ComponentProps<'input'>, 'onChange'> & { onChange(value: string): void }) => <input {...props} onChange={(event) => onChange(event.target.value)} />,
  Switch: ({ checked, ...props }: ComponentProps<'input'>) => <input {...props} type="checkbox" checked={checked} />,
  RawValueInput: ({ onChange, onUseLiteral, replacementLabel, kind, label, ...props }: RawValueInputProps) => <div data-raw-kind={kind}><input {...props} onChange={(event) => onChange(event.target.value)} /><button onClick={onUseLiteral}>Use {replacementLabel} for {label}</button></div>,
  SettingsList: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SettingsSection: ({ title, description, children }: { title: string; description?: ReactNode; children: ReactNode }) => <section><h3>{title}</h3>{description && <p>{description}</p>}{children}</section>,
  SettingsField: ({ field, label, description, error, layout, renderControl }: SettingsFieldProps) => {
    const id = `configuration-${field}`
    return <div data-settings-field={field} data-layout={layout}><label htmlFor={id}>{label}</label>{description && <p>{description}</p>}{error && <p role="alert">{error}</p>}{renderControl({ id, name: field, 'data-field': field, 'aria-invalid': error ? true : undefined })}</div>
  },
  StatusPanel: ({ headline, detail, action, variant: _variant, ...props }: StatusPanelProps) => <section {...props}>{headline}{detail}{action}</section>,
}))

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
const cleanups: Array<() => void> = []
const originalScrollIntoView = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollIntoView')
const applied: ConfigStatus = { id: 'layout-test', source: 'central', last_outcome: 'applied', last_error: null, rejected_reloads: 0, restart_required_fields: [], active: { ...DEFAULTS } }
const schema = { type: 'object', properties: { ...Object.fromEntries(Object.keys(DEFAULTS).map((key) => [key, {}])), cloud: { type: 'object' }, database: { type: 'object' } } }

async function render(overrides: Partial<ConfigFormProps> = {}, status: ConfigStatus | Error = applied) {
  const trigger = vi.fn(async () => {
    if (status instanceof Error) throw status
    return status
  })
  const host = { iii: { trigger, on: () => () => {}, registerTrigger: () => () => {}, browserId: 'test' } } as unknown as Host
  const container = document.createElement('div'); document.body.append(container)
  const root = createRoot(container)
  const props: ConfigFormProps = { id: 'layout-test', schema, value: { ...DEFAULTS, cloud: { ...CLOUD_DEFAULTS }, database: { name: 'whatsapp', namespace: 'my-project' } }, onChange: vi.fn(), ...overrides }
  await act(async () => root.render(<WhatsAppConfigForm host={host} {...props} />))
  cleanups.push(() => { act(() => root.unmount()); container.remove() })
  return {
    container, onChange: props.onChange,
    section(title: string) {
      const heading = Array.from(container.querySelectorAll('h3')).find((item) => item.textContent === title)
      expect(heading, `Missing settings section: ${title}`).toBeDefined()
      return heading!.closest('section')!
    },
    advanced() {
      const summary = Array.from(container.querySelectorAll('summary')).find((item) => item.textContent?.trim() === 'Advanced settings')
      expect(summary, 'Advanced settings disclosure must be present').toBeDefined()
      return summary!.closest('details')!
    },
    async update(next: Partial<ConfigFormProps>) { await act(async () => root.render(<WhatsAppConfigForm host={host} {...props} {...next} />)) },
    async type(field: string, value: string) {
      const input = container.querySelector<HTMLInputElement>(`input[data-field="${field}"]`)
      expect(input, `Missing input: ${field}`).not.toBeNull()
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value)
        input!.dispatchEvent(new Event('input', { bubbles: true }))
      })
    },
  }
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal('CSS', { escape: (value: string) => value })
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { configurable: true, value: vi.fn() })
})
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
  vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers()
  if (originalScrollIntoView) Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', originalScrollIntoView)
  else Reflect.deleteProperty(HTMLElement.prototype, 'scrollIntoView')
})

describe('WhatsApp configuration layout', () => {
  it('groups visible Meta account and webhook credentials before advanced settings', async () => {
    const view = await render()
    for (const [title, fields] of [
      ['Account', ['enabled', 'business_account_id', 'phone_number_id', 'access_token']],
      ['Webhook credentials', ['app_secret', 'verify_token']],
    ] as const) {
      const section = view.section(title)
      expect(Array.from(section.querySelectorAll('[data-field]')).map((item) => item.getAttribute('data-field'))).toEqual(fields.map((field) => `cloud.${field}`))
      for (const control of section.querySelectorAll('[data-field]')) expect(control.closest('details')).toBeNull()
      expect(section.closest('details')).toBeNull()
      expect(section.parentElement?.closest('section')).toBeNull()
    }
    expect(view.container.querySelector('.wa-ui-config-intro p')?.textContent).toBe('Meta credentials here; phone pairing in Linked phone.')
    expect(view.section('Meta Cloud API').textContent).toContain('Use your WhatsApp Business Platform account.')
    expect(view.container.querySelector('[data-settings-field="cloud.enabled"]')?.getAttribute('data-layout')).toBe('inline')
    const version = view.container.querySelector<HTMLInputElement>('[data-field="cloud.api_version"]')!
    expect(version.closest('details')).toBe(view.advanced())
    expect(version.value).toBe('v26.0')
    for (const field of ['access_token', 'app_secret', 'verify_token']) expect(view.container.querySelector<HTMLInputElement>(`[data-field="cloud.${field}"]`)!.type).toBe('password')
    expect(view.container.textContent).toContain('Linked phone')
    expect(view.container.textContent).toMatch(/QR/)
    expect(view.container.textContent).toMatch(/(?:no|without|does not (?:need|require)).*Meta credentials/i)
  })

  it('keeps official guide links visible and detailed retrieval help after the required controls', async () => {
    const view = await render()
    const account = view.section('Account')
    expect(account.textContent).toContain('Find the WABA and phone number IDs in WhatsApp Manager or your Meta app’s WhatsApp setup.')
    expect(account.textContent).toContain('Get a test access token there; create a system user token in Meta Business Settings for ongoing use.')
    const webhook = view.section('Webhook credentials')
    expect(webhook.textContent).toContain('Copy the app secret from App Settings > Basic in your Meta app.')
    expect(webhook.textContent).toContain('Choose your own verify token, then enter the same value here and in Meta’s webhook configuration.')
    for (const [section, href, label] of [
      [account, 'https://developers.facebook.com/documentation/business-messaging/whatsapp/get-started', 'Meta setup guide'],
      [webhook, 'https://developers.facebook.com/docs/graph-api/webhooks/getting-started', 'Meta webhook guide'],
    ] as const) {
      const link = section.querySelector('a')!
      expect(link.textContent).toBe(label)
      expect(link.href).toBe(href)
      expect(link.target).toBe('_blank')
      expect(link.rel).toBe('noopener noreferrer')
      expect(link.closest('details')).toBeNull()
      const help = section.querySelector<HTMLDetailsElement>('details.wa-ui-config-help')!
      expect(help.open).toBe(false)
      expect(help.querySelector('summary')?.textContent).toBe(section === account ? 'Find account credentials' : 'Find webhook credentials')
      for (const control of section.querySelectorAll('[data-field]')) {
        expect(control.closest('details')).toBeNull()
        expect(control.compareDocumentPosition(help) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
      }
    }
    expect(view.onChange).not.toHaveBeenCalled()
  })

  it('collapses advanced settings by default and separates database, storage, capacity and time fields', async () => {
    const view = await render()
    const advanced = view.advanced()
    expect(advanced.open).toBe(false)
    for (const title of ['Cloud API', 'Database worker', 'Storage & history', 'Capacity', 'Time limits']) expect(advanced.textContent).toContain(title)
    expect(Array.from(view.section('Database worker').querySelectorAll('[data-field]')).map((item) => item.getAttribute('data-field'))).toEqual(['database.name', 'database.namespace'])
    expect(Array.from(view.section('Storage & history').querySelectorAll('[data-field]')).map((item) => item.getAttribute('data-field'))).toEqual(['data_dir', 'max_events', 'max_inbox_messages'])
    for (const field of ['cloud.api_version', 'database.name', 'database.namespace', ...Object.keys(DEFAULTS)]) expect(advanced.querySelector(`[data-field="${field}"]`), `Missing advanced field: ${field}`).not.toBeNull()
    expect(advanced.querySelector<HTMLInputElement>('[data-field="database.name"]')?.value).toBe('whatsapp')
    expect(advanced.querySelector<HTMLInputElement>('[data-field="max_sessions"]')?.value).toBe('32')
  })

  it.each(['/cloud/api_version', '/database/name', '/max_sessions'])('opens advanced settings for validation errors in %s', async (path) => {
    const view = await render()
    expect(view.advanced().open).toBe(false)
    await view.update({ errors: new Map([[path, 'Review this value']]) })
    expect(view.advanced().open).toBe(true)
    expect(view.advanced().querySelector('[role="alert"]')?.textContent).toBe('Review this value')
  })

  it.each(['database.name', 'cloud.api_version'])('opens the disclosure before focusing %s', async (field) => {
    const view = await render()
    const advanced = view.advanced()
    expect(advanced.open).toBe(false)
    const focus = HTMLElement.prototype.focus
    const openWhenFocused: boolean[] = []
    vi.spyOn(HTMLElement.prototype, 'focus').mockImplementation(function (this: HTMLElement, options?: FocusOptions) {
      if (this.dataset.field === field) openWhenFocused.push(this.closest('details')?.open === true)
      focus.call(this, options)
    })
    await view.update({ focusField: field.split('.') })
    expect(advanced.open).toBe(true)
    expect(openWhenFocused).toEqual([true])
    expect(document.activeElement).toBe(view.container.querySelector(`[data-field="${field}"]`))
  })

  it('keeps runtime overrides visible and the environment reference complete', async () => {
    const view = await render({}, { ...applied, environment_overrides: { 'cloud.api_version': 'WHATSAPP_CLOUD_API_VERSION' } })
    const notice = Array.from(view.container.querySelectorAll('section')).find((item) => item.textContent?.includes('Cloud API uses environment overrides'))!
    expect(notice.closest('details')).toBeNull()
    expect(notice.getAttribute('role')).toBe('status')
    expect(view.advanced().open).toBe(false)
    const reference = view.container.querySelector('.wa-ui-config-environment:not(.wa-ui-config-help)')!
    expect(Array.from(reference.querySelectorAll('code')).map((item) => item.textContent)).toEqual([
      'WHATSAPP_CLOUD_ENABLED', 'WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID', 'WHATSAPP_CLOUD_PHONE_NUMBER_ID',
      'WHATSAPP_CLOUD_ACCESS_TOKEN', 'WHATSAPP_META_APP_SECRET', 'WHATSAPP_WEBHOOK_VERIFY_TOKEN', 'WHATSAPP_CLOUD_API_VERSION',
    ])
  })

  it.each([
    { status: applied, role: 'status', headline: 'Active configuration:' },
    { status: { ...applied, last_outcome: 'restart_required' as const, restart_required_fields: ['data_dir'] }, role: 'status', headline: 'Some changes need a worker restart' },
    { status: { ...applied, source: 'local' as const }, role: 'status', headline: 'Local configuration is active' },
    { status: { ...applied, last_outcome: 'rejected' as const, rejected_reloads: 1 }, role: 'alert', headline: 'Latest configuration rejected' },
    { status: new Error('Runtime unavailable'), role: 'alert', headline: 'Active configuration unavailable' },
  ])('exposes the $headline runtime result as a $role', async ({ status, role, headline }) => {
    const view = await render({}, status)
    expect(view.container.querySelector(`[role="${role}"]`)?.textContent).toContain(headline)
  })

  it('preserves environment references, masked secrets, nullable and custom fields when editing the advanced version', async () => {
    const value = {
      ...DEFAULTS, future: { retry: true },
      database: { name: '${DATABASE_POOL}', namespace: null, future: 'keep' },
      cloud: { ...CLOUD_DEFAULTS, enabled: '${CLOUD_ENABLED}', phone_number_id: null, access_token: '${META_TOKEN}', app_secret: { secret: 'preserve-this' }, api_version: '${GRAPH_VERSION}', future: { request: true } },
    }
    const view = await render({ value })
    const version = view.container.querySelector<HTMLInputElement>('[data-field="cloud.api_version"]')!
    expect(version.value).toBe('${GRAPH_VERSION}')
    expect(view.container.querySelector<HTMLInputElement>('[data-field="cloud.access_token"]')!.type).toBe('password')
    expect(view.container.querySelector('[data-settings-field="cloud.enabled"]')?.getAttribute('data-layout')).toBe('auto')
    expect(view.container.textContent).not.toContain('preserve-this')
    expect(view.onChange).not.toHaveBeenCalled()
    await view.update({ focusField: ['cloud', 'api_version'] })
    expect(view.advanced().open).toBe(true)
    expect(document.activeElement).toBe(version)
    expect(version.value).toBe('${GRAPH_VERSION}')
    await view.type('cloud.api_version', 'v27.0')
    expect(view.onChange).toHaveBeenCalledExactlyOnceWith({ ...value, cloud: { ...value.cloud, api_version: 'v27.0' } })
  })

  it.each([null, '${CLOUD_CONFIG}', [1], 3])('keeps an opaque Cloud configuration editable without materializing advanced fields: %s', async (cloud) => {
    const view = await render({ value: { ...DEFAULTS, cloud } })
    const control = view.container.querySelector<HTMLElement>('[data-field="cloud"]')!
    expect(control).not.toBeNull()
    expect(view.container.querySelector('[data-field="cloud.api_version"]')).toBeNull()
    expect(view.container.querySelector('[data-field="cloud.enabled"]')).toBeNull()
    expect(view.onChange).not.toHaveBeenCalled()
    await view.update({ focusField: ['cloud', 'api_version'] })
    expect(document.activeElement).toBe(control)
    expect(view.advanced().open).toBe(false)
  })
})

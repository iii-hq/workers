// biome-ignore-all lint/suspicious/noTemplateCurlyInString: these literals intentionally exercise ${VAR} configuration templates.
import type { ComponentProps, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { BrowserConfigEditor, browserConfigurationValue, focusBrowserNarrowPane, migrateBrowserConfiguration } from '.'
import { booleanLiteralForRawValue, numberLiteralForRawValue } from './template-values'

vi.mock('@iii-dev/console-ui', () => ({
  Input: ({ preserveCase: _preserveCase, onChange: _onChange, ...props }: Record<string, unknown>) => (
    <input {...props} />
  ),
  SettingsList: ({ children, ...props }: { children?: ReactNode }) => <div {...props}>{children}</div>,
  SettingsSection: ({
    title,
    description,
    children,
    ...props
  }: {
    title?: ReactNode
    description?: ReactNode
    children?: ReactNode
  }) => (
    <section {...props}>
      {title}
      {description}
      {children}
    </section>
  ),
  SettingsRow: ({
    label,
    description,
    meta,
    control,
    className,
  }: {
    label: ReactNode
    description?: ReactNode
    meta?: ReactNode
    control?: ReactNode
    className?: string
  }) => (
    <div className={className}>
      {label}
      {description}
      {meta}
      {control}
    </div>
  ),
  StatusPanel: () => null,
  ConfirmDialog: () => null,
  Switch: (props: ComponentProps<'input'>) => <input {...props} type="checkbox" />,
}))

const value = {
  executable: '/opt/chrome',
  data_dir: './data/browser',
  headless: true,
  max_sessions: 4,
  console_buffer: 500,
  network_buffer: 500,
  viewport_width: 1280,
  viewport_height: 800,
  default_timeout_ms: 30_000,
  max_timeout_ms: 120_000,
  inactive_after_ms: 1_800_000,
  screenshot_quality: 60,
  allowed_schemes: ['http', 'https', 'file'],
  max_snapshot_nodes: 2_000,
  default_origin_policy: { access: 'deny' },
  origin_policies: {
    'https://app.example.com': {
      access: 'allow',
      downloads: 'deny',
      uploads: 'deny',
      scripting: 'allow',
    },
  },
  allow_history_access: true,
  allow_cookie_import: true,
  allow_attach: false,
  scrapling: {
    security_mode: 'safe',
    chromium_executable: '/opt/scraping-chrome',
    allow_loopback: false,
    defaults: {
      impersonate: 'chrome',
      headless: true,
      network_idle: false,
      proxy: '',
      include_html: false,
    },
    max_bulk_concurrency: 5,
    max_sessions: 8,
    session_idle_timeout_s: 900,
    adaptive_storage_path: './data/scrapling/elements.db',
    adaptive_max_bytes: 268_435_456,
    inject_guidance: true,
  },
}

function renderSection(selection: Parameters<typeof BrowserConfigEditor>[0]['selection']) {
  return renderToStaticMarkup(
    <BrowserConfigEditor
      selection={selection}
      value={value}
      errors={new Map()}
      narrow={false}
      onBack={() => {}}
      onChange={() => {}}
    />,
  )
}

describe('BrowserConfigEditor schema parity', () => {
  it('renders every public top-level configuration field', () => {
    const html = [
      renderSection('launch'),
      renderSection('viewport'),
      renderSection('limits'),
      renderSection('behavior'),
      renderSection('access'),
      renderSection('scraping'),
    ].join('')

    for (const field of [
      'executable',
      'data_dir',
      'headless',
      'max_sessions',
      'console_buffer',
      'network_buffer',
      'viewport_width',
      'viewport_height',
      'default_timeout_ms',
      'max_timeout_ms',
      'inactive_after_ms',
      'screenshot_quality',
      'allowed_schemes',
      'max_snapshot_nodes',
      'default_origin_policy',
      'origin_policies',
      'allow_history_access',
      'allow_cookie_import',
      'allow_attach',
      'scrapling',
    ]) {
      expect(html).toContain(`data-field="${field}`)
    }
  })

  it('renders every Scrapling setting as a dedicated control', () => {
    const html = renderSection('scraping')
    for (const field of [
      'scrapling.security_mode',
      'scrapling.chromium_executable',
      'scrapling.allow_loopback',
      'scrapling.defaults.impersonate',
      'scrapling.defaults.headless',
      'scrapling.defaults.network_idle',
      'scrapling.defaults.proxy',
      'scrapling.defaults.include_html',
      'scrapling.max_bulk_concurrency',
      'scrapling.max_sessions',
      'scrapling.session_idle_timeout_s',
      'scrapling.adaptive_storage_path',
      'scrapling.adaptive_max_bytes',
      'scrapling.inject_guidance',
    ]) {
      expect(html).toContain(`data-field="${field}"`)
    }
  })

  it('renders separate decisions for fallback and per-origin policies', () => {
    const html = renderSection('access')
    for (const capability of ['access', 'downloads', 'uploads', 'scripting']) {
      expect(html).toContain(`data-field="default_origin_policy.${capability}"`)
      expect(html).toContain(`data-field="origin_policies.https://app.example.com.${capability}"`)
    }
  })

  it('reads and atomically migrates a legacy browser envelope', () => {
    const legacy = {
      executable: '/stale/flat/chrome',
      deployment: { keep: true },
      browser: {
        ...value,
        executable: '/legacy/chrome',
        future_browser_setting: { keep: true },
      },
    }

    const editable = browserConfigurationValue(legacy)
    expect(editable.executable).toBe('/legacy/chrome')

    const migrated = migrateBrowserConfiguration(legacy, {
      ...editable,
      executable: '/edited/chrome',
    })
    expect(migrated).not.toHaveProperty('browser')
    expect(migrated.executable).toBe('/edited/chrome')
    expect(migrated.deployment).toEqual({ keep: true })
    expect(migrated.future_browser_setting).toEqual({ keep: true })
  })

  it('renders wrapped browser settings instead of stale flat siblings', () => {
    const html = renderToStaticMarkup(
      <BrowserConfigEditor
        selection="launch"
        value={browserConfigurationValue({
          executable: '/stale/flat/chrome',
          browser: { ...value, executable: '/legacy/chrome' },
        })}
        errors={new Map()}
        narrow={false}
        onBack={() => {}}
        onChange={() => {}}
      />,
    )
    expect(html).toContain('value="/legacy/chrome"')
    expect(html).not.toContain('/stale/flat/chrome')
  })

  it('keeps number and boolean environment templates visible until explicitly replaced', () => {
    const html = renderToStaticMarkup(
      <BrowserConfigEditor
        selection="launch"
        value={{
          ...value,
          max_sessions: '${MAX_SESSIONS:6}',
          headless: '${HEADLESS:false}',
        }}
        errors={new Map()}
        narrow={false}
        onBack={() => {}}
        onChange={() => {}}
      />,
    )

    expect(html).toContain('value="${MAX_SESSIONS:6}"')
    expect(html).toContain('value="${HEADLESS:false}"')
    expect(html.match(/data-environment-template="true"/g)).toHaveLength(2)
    expect(html).toContain('Use 6')
    expect(html).toContain('Use off')
    expect(numberLiteralForRawValue('${MAX_SESSIONS:6}', 4)).toBe(6)
    expect(booleanLiteralForRawValue('${HEADLESS:false}', true)).toBe(false)
  })

  it('transfers focus into the narrow editor and restores it to the selected navigation row', () => {
    const editor = { focus: vi.fn(), scrollIntoView: vi.fn() }
    const nav = { focus: vi.fn(), scrollIntoView: vi.fn() }
    const root = {
      querySelector: vi.fn((selector: string) => (selector === '.br-cfg-editor' ? editor : nav)),
    }

    expect(focusBrowserNarrowPane(root as unknown as HTMLElement, 'editor', 'access')).toBe(true)
    expect(editor.focus).toHaveBeenCalledWith({ preventScroll: true })
    expect(editor.scrollIntoView).toHaveBeenCalledWith({ block: 'nearest' })

    expect(focusBrowserNarrowPane(root as unknown as HTMLElement, 'nav', 'access')).toBe(true)
    expect(root.querySelector).toHaveBeenLastCalledWith('[data-config-section="access"]')
    expect(nav.focus).toHaveBeenCalledWith({ preventScroll: true })
    expect(nav.scrollIntoView).toHaveBeenCalledWith({ block: 'nearest' })
  })
})

// biome-ignore-all lint/suspicious/noTemplateCurlyInString: these literals intentionally exercise ${VAR} configuration templates.
import type { Host, JsonValue } from '@iii-dev/console-ui'
import type { ComponentProps, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { DatabaseConfigForm } from '.'

vi.mock('@iii-dev/console-ui', () => ({
  Button: ({ children, ...props }: ComponentProps<'button'>) => <button {...props}>{children}</button>,
  Chip: ({ children, tone: _tone, ...props }: ComponentProps<'span'> & { tone?: string }) => (
    <span {...props}>{children}</span>
  ),
  Input: ({
    onChange: _onChange,
    ...props
  }: { onChange?: (next: string) => void } & Omit<ComponentProps<'input'>, 'onChange'>) => (
    <input data-shared-input="" readOnly {...props} />
  ),
  DropdownMenu: ({ children }: { children?: ReactNode }) => <div data-dropdown="">{children}</div>,
  DropdownMenuTrigger: ({ children }: { children?: ReactNode; asChild?: boolean }) => <>{children}</>,
  DropdownMenuContent: ({ children }: { children?: ReactNode; align?: string }) => (
    <div data-dropdown-content="">{children}</div>
  ),
  DropdownMenuItem: ({ children, onSelect: _onSelect }: { children?: ReactNode; onSelect?: () => void }) => (
    <div role="menuitem">{children}</div>
  ),
  List: ({ children, ...props }: ComponentProps<'div'>) => <div {...props}>{children}</div>,
  ListItem: ({
    leading,
    label,
    description,
    trailing,
    ...props
  }: ComponentProps<'button'> & {
    leading?: ReactNode
    label?: ReactNode
    description?: ReactNode
    trailing?: ReactNode
  }) => (
    <button {...props}>
      {leading}
      {label}
      {description}
      {trailing}
    </button>
  ),
  Panel: ({ children, ...props }: ComponentProps<'div'>) => <div {...props}>{children}</div>,
  SegmentedControl: ({
    value,
    options,
    'aria-label': ariaLabel,
  }: {
    value: string
    options: Array<{ value: string; label: ReactNode }>
    'aria-label'?: string
  }) => (
    <div role="radiogroup" aria-label={ariaLabel} data-segmented-value={value}>
      {options.map((option) => (
        <button key={option.value} type="button" role="radio" aria-checked={option.value === value}>
          {option.label}
        </button>
      ))}
    </div>
  ),
  RawValueInput: ({
    label: _label,
    kind,
    replacementLabel,
    onUseLiteral: _onUseLiteral,
    onChange: _onChange,
    inputClassName: _inputClassName,
    ...props
  }: {
    label: string
    kind: string
    replacementLabel: ReactNode
    onUseLiteral: () => void
    onChange: (next: string) => void
    inputClassName?: string
  } & Omit<ComponentProps<'input'>, 'onChange'>) => (
    <div data-raw-value-kind={kind}>
      <input data-shared-input="" readOnly {...props} />
      <button type="button">Use {replacementLabel}</button>
    </div>
  ),
  Select: ({
    value,
    options,
    onChange: _onChange,
    name: _name,
    sheetTitle: _sheetTitle,
    ...props
  }: {
    value?: string
    options?: Array<{ value: string; label: string }>
    onChange: (next: string) => void
    name?: string
    sheetTitle?: ReactNode
  } & ComponentProps<'button'>) => (
    <button data-shared-select="" {...props}>
      {options?.find((option) => option.value === value)?.label ?? value}
    </button>
  ),
  SettingsDeck: ({
    open,
    overview,
    detail,
    title,
    backLabel,
  }: {
    open: boolean
    overview: ReactNode
    detail: ReactNode
    title: ReactNode
    backLabel?: ReactNode
  }) => (
    <div data-state={open ? 'detail' : 'overview'}>
      {open ? (
        <section>
          <button type="button">{backLabel}</button>
          <h2>{title}</h2>
          {detail}
        </section>
      ) : (
        overview
      )}
    </div>
  ),
  SettingsField: ({
    id,
    field,
    label,
    description,
    meta,
    error,
    renderControl,
  }: {
    id: string
    field?: string
    label: ReactNode
    description?: ReactNode
    meta?: ReactNode
    error?: ReactNode
    renderControl: (props: Record<string, unknown>) => ReactNode
  }) => (
    <div>
      <label htmlFor={id}>{label}</label>
      {description}
      {meta}
      {error}
      {renderControl({ id, name: field, 'data-field': field })}
    </div>
  ),
  SettingsList: ({ children, ...props }: { children?: ReactNode }) => <div {...props}>{children}</div>,
  SettingsSection: ({
    title,
    description,
    action,
    children,
    ...props
  }: {
    title?: ReactNode
    description?: ReactNode
    action?: ReactNode
    children?: ReactNode
  }) => (
    <section {...props}>
      {title}
      {description}
      {action}
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
  Switch: (props: ComponentProps<'input'>) => <input {...props} type="checkbox" />,
}))

const host = { iii: { trigger: vi.fn() } } as unknown as Host

const DETAIL_FOCUS = ['databases', 'primary', 'url']

function renderDatabase(url: string, focusField?: string[]) {
  return renderDatabaseValue({ databases: { primary: { url } } }, focusField)
}

/** The shape of the worker's registered schema, reduced to what the form reads. */
const STRUCTURED_SCHEMA = {
  definitions: {
    DatabaseConfig: { type: 'object', properties: { url: {}, driver: {}, host: {}, ssh: {}, tls: {}, display: {} } },
  },
}

function renderDatabaseValue(
  value: JsonValue,
  focusField?: string[],
  schema: Record<string, unknown> | null = STRUCTURED_SCHEMA,
) {
  return renderToStaticMarkup(
    <DatabaseConfigForm
      id="database"
      schema={schema}
      value={value}
      errors={new Map()}
      focusField={focusField}
      host={host}
      onChange={() => {}}
    />,
  )
}

describe('DatabaseConfigForm', () => {
  it('starts on the connections overview instead of preselecting the first database', () => {
    const html = renderDatabase('sqlite:./data/dev.db', undefined)

    expect(html).toContain('data-state="overview"')
    expect(html).toContain('Configure database primary')
    expect(html).not.toContain('Connection settings')
  })

  it('keeps the empty-state action reachable in a narrow settings pane', () => {
    const html = renderDatabaseValue({ databases: {} })

    expect(html).toContain('No databases configured')
    expect(html).toContain('data-settings-narrow-action="true"')
  })

  it('preserves an opaque root until an explicit conversion', () => {
    const template = renderDatabaseValue('${DATABASE_CONFIG}')
    const futureShape = renderDatabaseValue(['future-database-config'])

    expect(template).toContain('value="${DATABASE_CONFIG}"')
    expect(template).toContain('Use SQLite defaults')
    expect(template).not.toContain('Query history')
    expect(futureShape).toContain('Custom value preserved')
    expect(futureShape).toContain('Use SQLite defaults')
  })

  it('keeps TLS and pool settings available for a whole URL template with a SQLite default', () => {
    const html = renderDatabase('${DATABASE_URL:sqlite:./data/dev.db}', DETAIL_FOCUS)

    expect(html).toContain('value="${DATABASE_URL:sqlite:./data/dev.db}"')
    expect(html).toContain('Transport security')
    expect(html).toContain('Connection pool')
    expect(html).not.toContain('Show connection URL')
    expect(html).toContain('data-shared-input=""')
    expect(html).toContain('data-shared-select=""')
  })

  it('masks a network URL that contains a partial template', () => {
    const html = renderDatabase('postgres://admin:literal-secret@db/app?application_name=${APP_NAME}', DETAIL_FOCUS)

    expect(html).toMatch(/data-field="databases\.primary\.url"[^>]*type="password"/)
    expect(html).toContain('aria-label="Show connection URL"')
    expect(html).toContain('Transport security')
  })

  it('opens the requested connection as a dedicated deck level', () => {
    const html = renderDatabase('sqlite:./data/dev.db', DETAIL_FOCUS)

    expect(html).toContain('data-state="detail"')
    expect(html).toContain('>Connections</button>')
    expect(html).toContain('<h2>primary</h2>')
  })

  it('keeps opaque collection, connection, TLS, and pool values explicit', () => {
    const collection = renderDatabaseValue({
      databases: '${DATABASE_CONNECTIONS}',
    })
    const connection = renderDatabaseValue({ databases: { primary: '${PRIMARY_DATABASE}' } }, ['databases', 'primary'])
    const blocks = renderDatabaseValue(
      {
        databases: {
          primary: {
            url: 'postgres://db/app',
            tls: '${DATABASE_TLS}',
            pool: '${DATABASE_POOL}',
          },
        },
      },
      DETAIL_FOCUS,
    )

    expect(collection).toContain('value="${DATABASE_CONNECTIONS}"')
    expect(connection).toContain('value="${PRIMARY_DATABASE}"')
    expect(blocks).toContain('value="${DATABASE_TLS}"')
    expect(blocks).toContain('value="${DATABASE_POOL}"')
    expect(blocks).not.toContain('Maximum connections')
  })

  it('requires an explicit conversion for non-string opaque values', () => {
    const connection = renderDatabaseValue({ databases: { primary: 42 } }, ['databases', 'primary'])
    const blocks = renderDatabaseValue(
      {
        databases: {
          primary: {
            url: 'postgres://db/app',
            tls: ['future-tls-shape'],
            pool: true,
          },
        },
      },
      DETAIL_FOCUS,
    )

    expect(connection).toContain('Custom value preserved')
    expect(connection).toContain('Use SQLite defaults')
    expect(blocks).toContain('Use TLS defaults')
    expect(blocks).toContain('Use pool defaults')
    expect(blocks).not.toContain('Maximum connections')
  })

  describe('structured connections', () => {
    const SHOP_FOCUS = ['databases', 'shop', 'host']
    const shop = (extra: Record<string, JsonValue> = {}): JsonValue => ({
      databases: {
        shop: {
          driver: 'postgres',
          host: 'db.internal',
          port: 6543,
          user: 'app',
          password: 'secret://DATABASE_SHOP_PASSWORD',
          database: 'shop',
          ...extra,
        },
      },
    })

    it('starts with the driver, then the endpoint fields', () => {
      const html = renderDatabaseValue(shop(), SHOP_FOCUS)

      expect(html).toContain('role="radiogroup" aria-label="Driver" data-segmented-value="postgres"')
      expect(html.indexOf('aria-label="Driver"')).toBeLessThan(html.indexOf('data-field="databases.shop.host"'))
      expect(html).toContain('value="db.internal"')
      expect(html).toContain('value="6543"')
      expect(html).toContain('value="app"')
      expect(html).toContain('Test connection')
      expect(html).not.toContain('Connection URL')
    })

    it('shows a stored credential by reference, never as a value', () => {
      const html = renderDatabaseValue(shop(), SHOP_FOCUS)

      expect(html).toContain('secret://DATABASE_SHOP_PASSWORD')
      expect(html).toContain('Reading the secrets store…')
      expect(html).toContain('>Replace</button>')
    })

    it('flags a plain-text password and offers to move it', () => {
      const html = renderDatabaseValue(shop({ password: 'hunter2' }), SHOP_FOCUS)

      expect(html).toContain('Plain text')
      expect(html).toContain('Move to secrets')
      expect(html).not.toContain('hunter2')
    })

    it('takes a new password into the secrets worker under the derived name', () => {
      const html = renderDatabaseValue(shop({ password: '' }), SHOP_FOCUS)

      expect(html).toMatch(/data-field="databases\.shop\.password"[^>]*type="password"/)
      expect(html).toContain('Stored as secret://DATABASE_SHOP_PASSWORD')
      expect(html).toContain('Save to secrets')
    })

    it('asks for a file path, not a server, for SQLite', () => {
      const html = renderDatabaseValue(
        { databases: { shop: { driver: 'sqlite', database: './data/shop.db' } } },
        ['databases', 'shop', 'database'],
      )

      expect(html).toContain('Database file')
      expect(html).toContain('value="./data/shop.db"')
      expect(html).not.toContain('data-field="databases.shop.host"')
      expect(html).not.toContain('Transport security')
      expect(html).not.toContain('SSH tunnel')
    })

    it('offers icon, colour and the queries folder', () => {
      const html = renderDatabaseValue(
        shop({ display: { icon: 'server', color: '#16A34A' }, folder: '/srv/queries' }),
        SHOP_FOCUS,
      )

      expect(html).toContain('aria-label="server"')
      expect(html).toMatch(/aria-checked="true" aria-label="server"/)
      expect(html).toMatch(/aria-checked="true" aria-label="#16A34A"/)
      expect(html).toContain('value="/srv/queries"')
    })

    it('opens the SSH tunnel fields only when the tunnel is on', () => {
      const off = renderDatabaseValue(shop(), SHOP_FOCUS)
      const on = renderDatabaseValue(
        shop({
          ssh: {
            host: 'bastion',
            user: 'ubuntu',
            private_key: 'secret://DATABASE_SHOP_SSH_PRIVATE_KEY',
            host_key: 'SHA256:abc',
          },
        }),
        SHOP_FOCUS,
      )

      expect(off).toContain('Connect through an SSH tunnel')
      expect(off).not.toContain('data-field="databases.shop.ssh.host"')
      expect(on).toContain('value="bastion"')
      expect(on).toContain('secret://DATABASE_SHOP_SSH_PRIVATE_KEY')
      expect(on).toContain('value="SHA256:abc"')
      expect(on).toContain('Key passphrase')
    })

    it('explains the TLS mode and takes client certificates', () => {
      const html = renderDatabaseValue(shop({ tls: { mode: 'verify-ca', client_key: 'secret://K' } }), SHOP_FOCUS)

      expect(html).toContain('TLS required and the chain validated against the trusted CAs')
      expect(html).toContain('Client certificate')
      expect(html).toContain('secret://K')
      expect(html).toContain('Paste PEM')
    })

    it('keeps a legacy url editable and offers the conversion when it is lossless', () => {
      const convertible = renderDatabase('postgres://app:pw@db/app', DETAIL_FOCUS)
      const template = renderDatabase('${DATABASE_URL}', DETAIL_FOCUS)

      expect(convertible).toContain('Connection URL')
      expect(convertible).toMatch(/data-field="databases\.primary\.convert"(?![^>]*disabled)/)
      expect(template).toMatch(/data-field="databases\.primary\.convert"[^>]*disabled/)
      expect(template).toContain('The URL is filled from the environment.')
    })

    it('keeps the url-only form for a worker whose schema has no structured entries', () => {
      const legacySchema = {
        definitions: { DatabaseConfig: { type: 'object', properties: { url: {}, tls: {}, pool: {} } } },
      }
      const html = renderDatabaseValue({ databases: { primary: { url: 'postgres://db/app' } } }, DETAIL_FOCUS, legacySchema)
      const overview = renderDatabaseValue({ databases: { primary: { url: 'postgres://db/app' } } }, undefined, legacySchema)

      expect(html).toContain('Connection URL')
      expect(html).not.toContain('Convert to fields')
      expect(html).not.toContain('SSH tunnel')
      expect(html).not.toContain('Queries folder')
      expect(html).toContain('Extra CA bundle')
      expect(overview).not.toContain('role="menuitem"')
    })

    it('lists connections with their glyph and endpoint, and adds one by driver', () => {
      const html = renderDatabaseValue(shop({ display: { icon: 'server', color: '#16A34A' } }))

      expect(html).toContain('db.internal:6543/shop')
      expect(html).toContain('db-cfg-conn-tile')
      expect(html).toContain('color:#16A34A')
      expect(html).toContain('role="menuitem">PostgreSQL<')
      expect(html).toContain('role="menuitem">SQLite<')
    })
  
    it('keeps a key file path as a path, not a secret', () => {
      const html = renderDatabaseValue(
        shop({ ssh: { host: 'bastion', user: 'ubuntu', private_key: '/home/me/.ssh/id_ed25519' } }),
        SHOP_FOCUS,
      )

      expect(html).toContain('/home/me/.ssh/id_ed25519')
      expect(html).toContain('A file on the engine host')
      expect(html).not.toContain('Move to secrets')
    })

    it('says what the worker would refuse before saving', () => {
      const html = renderDatabaseValue(
        shop({ tls: { client_cert: '/etc/ssl/c.crt' }, ssh: { host: 'b', user: 'u' }, folder: 'queries' }),
        SHOP_FOCUS,
      )

      expect(html).toContain('Mutual TLS needs both the client certificate and its key.')
      expect(html).toContain('The tunnel needs a password or a private key.')
      expect(html).toContain('Use an absolute path on the engine host.')
    })
  
    it('without a schema, trusts entries already stored in the structured shape', () => {
      const structured = renderDatabaseValue(shop(), SHOP_FOCUS, null)
      const legacy = renderDatabaseValue({ databases: { primary: { url: 'postgres://db/app' } } }, DETAIL_FOCUS, null)

      expect(structured).toContain('aria-label="Driver"')
      expect(structured).toContain('SSH tunnel')
      // Until the engine's function list answers, a url-only collection keeps the url-only form.
      expect(legacy).not.toContain('Convert to fields')
    })
  })
})

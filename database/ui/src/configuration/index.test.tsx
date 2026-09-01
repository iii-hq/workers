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
  List: ({ children, ...props }: ComponentProps<'div'>) => <div {...props}>{children}</div>,
  ListItem: ({
    label,
    description,
    trailing,
    ...props
  }: ComponentProps<'button'> & {
    label?: ReactNode
    description?: ReactNode
    trailing?: ReactNode
  }) => (
    <button {...props}>
      {label}
      {description}
      {trailing}
    </button>
  ),
  Panel: ({ children, ...props }: ComponentProps<'div'>) => <div {...props}>{children}</div>,
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

function renderDatabaseValue(value: JsonValue, focusField?: string[]) {
  return renderToStaticMarkup(
    <DatabaseConfigForm
      id="database"
      schema={null}
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
})

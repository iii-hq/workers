// biome-ignore-all lint/suspicious/noTemplateCurlyInString: these literals intentionally exercise ${VAR} configuration templates.
import type { Host } from '@iii-dev/console-ui'
import type { ComponentProps, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { DatabaseConfigForm, focusDatabaseDetail } from '.'

vi.mock('@iii-dev/console-ui', () => ({
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

function renderDatabase(url: string) {
  return renderToStaticMarkup(
    <DatabaseConfigForm
      id="database"
      schema={null}
      value={{ databases: { primary: { url } } }}
      errors={new Map()}
      focusField={undefined}
      host={host}
      onChange={() => {}}
    />,
  )
}

describe('DatabaseConfigForm', () => {
  it('keeps TLS and pool settings available for a whole URL template with a SQLite default', () => {
    const html = renderDatabase('${DATABASE_URL:sqlite:./data/dev.db}')

    expect(html).toContain('value="${DATABASE_URL:sqlite:./data/dev.db}"')
    expect(html).toContain('Transport security')
    expect(html).toContain('Connection pool')
    expect(html).not.toContain('Show connection URL')
  })

  it('masks a network URL that contains a partial template', () => {
    const html = renderDatabase('postgres://admin:literal-secret@db/app?application_name=${APP_NAME}')

    expect(html).toMatch(/data-field="databases\.primary\.url"[^>]*type="password"/)
    expect(html).toContain('aria-label="Show connection URL"')
    expect(html).toContain('Transport security')
  })

  it('focuses and scrolls the active detail after narrow master selection', () => {
    const target = {
      focus: vi.fn(),
      scrollIntoView: vi.fn(),
    }
    const root = {
      querySelector: vi.fn(() => target),
    }

    expect(focusDatabaseDetail(root as unknown as HTMLElement)).toBe(true)
    expect(root.querySelector).toHaveBeenCalledWith('.db-cfg-detail')
    expect(target.focus).toHaveBeenCalledWith({ preventScroll: true })
    expect(target.scrollIntoView).toHaveBeenCalledWith({ block: 'start' })
  })
})

import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { DirectoryConfigForm } from './index'

// Injected worker UIs receive this package from the Console import map. The
// package deliberately throws when loaded directly, so the test mirrors only
// the documented rendering contract needed by this form.
vi.mock('@iii-dev/console-ui', () => ({
  Chip: ({ children }: { children?: ReactNode }) => <span>{children}</span>,
  Input: ({ onChange: _onChange, ...props }: { onChange?: unknown }) => <input {...props} />,
  Select: ({
    onChange: _onChange,
    options,
    value,
    ...props
  }: {
    onChange?: unknown
    options: Array<{ label: string; value: string }>
    value?: string
  }) => (
    <select {...props} defaultValue={value}>
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  ),
  SettingsList: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  SettingsRow: ({
    label,
    description,
    meta,
    control,
    ...props
  }: {
    label?: ReactNode
    description?: ReactNode
    meta?: ReactNode
    control?: ReactNode
  }) => (
    <div {...props}>
      {label}
      {description}
      {meta}
      {control}
    </div>
  ),
  SettingsSection: ({
    title,
    description,
    children,
  }: {
    title?: ReactNode
    description?: ReactNode
    children?: ReactNode
  }) => (
    <section>
      <h2>{title}</h2>
      <p>{description}</p>
      {children}
    </section>
  ),
  Switch: ({ onChange: _onChange, ...props }: { onChange?: unknown }) => <input type="checkbox" {...props} />,
}))

function renderConfiguration(value: Record<string, string | null>) {
  return renderToStaticMarkup(
    <DirectoryConfigForm id="iii-directory" schema={{}} value={value} onChange={() => {}} errors={new Map()} />,
  )
}

describe('DirectoryConfigForm function search settings', () => {
  it('renders hybrid as the default without a model notice', () => {
    const html = renderConfiguration({})

    expect(html).toContain('Function search mode')
    expect(html).toContain('<option value="hybrid" selected="">')
    expect(html).not.toContain('requires a local semantic model')
  })

  it('shows how to recover when the model directory is cleared', () => {
    const html = renderConfiguration({ function_search_mode: 'hybrid', function_search_model_path: null })

    expect(html).toContain('Hybrid')
    expect(html).toContain('requires a local semantic model')
    expect(html).toContain('function_search_model_path')
    expect(html).toContain('Restart required')
  })
})

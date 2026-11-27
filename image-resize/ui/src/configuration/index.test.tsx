import type { JsonValue } from '@iii-dev/console-ui'
import type { ComponentProps, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { ImageResizeConfigForm, settingNumber, withPath } from './index'

vi.mock('@iii-dev/console-ui', () => ({
  Input: ({ onChange: _onChange, ...props }: { onChange?: unknown } & Omit<ComponentProps<'input'>, 'onChange'>) => (
    <input readOnly {...props} />
  ),
  Select: ({
    value,
    placeholder,
    'data-field': field,
  }: {
    value?: string
    placeholder?: string
    'data-field'?: string
  }) => <div data-field={field} data-select={value ?? placeholder} />,
  SettingsField: ({
    label,
    error,
    field,
    renderControl,
  }: {
    label: string
    error?: string
    field: string
    renderControl: (props: { id: string; 'data-field': string }) => ReactNode
  }) => (
    <div data-row={field}>
      <label htmlFor={field}>{label}</label>
      {error ? <p role="alert">{error}</p> : null}
      {renderControl({ id: field, 'data-field': field })}
    </div>
  ),
  SettingsList: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  SettingsSection: ({ children, title }: { children?: ReactNode; title: string }) => (
    <section>
      <h2>{title}</h2>
      {children}
    </section>
  ),
}))

const render = (value: JsonValue, errors?: Map<string, string>) =>
  renderToStaticMarkup(
    <ImageResizeConfigForm id="image-resize" schema={null} value={value} onChange={() => {}} errors={errors} />,
  )

describe('ImageResizeConfigForm', () => {
  it('renders the seven fields with their labels, values and errors', () => {
    const html = render(
      { width: 640, strategy: 'crop-to-fit', quality: { webp: 70 }, max_bytes: 500000 },
      new Map([['/quality/jpeg', 'quality must be 1-100, got 0']]),
    )
    expect(html).toContain('<h2>Default box</h2>')
    expect(html).toContain('<h2>Default output</h2>')
    expect(html).toContain('<h2>Output limits</h2>')
    for (const [field, label] of [
      ['width', 'Width'],
      ['height', 'Height'],
      ['strategy', 'Strategy'],
      ['quality.jpeg', 'JPEG quality'],
      ['quality.webp', 'WebP quality'],
      ['max_pixels', 'Max pixels'],
      ['max_bytes', 'Max bytes'],
    ]) {
      expect(html).toContain(`<label for="${field}">${label}</label>`)
      expect(html).toContain(`data-field="${field}"`)
    }
    expect(html).toMatch(/data-field="width"[^>]*value="640"|value="640"[^>]*data-field="width"/)
    expect(html).toMatch(/placeholder="200"[^>]*data-field="height"|data-field="height"[^>]*placeholder="200"/)
    expect(html).toContain('data-select="crop-to-fit"')
    expect(html).toContain('placeholder="85"')
    expect(html).toContain('value="70"')
    expect(html).toMatch(/data-field="max_bytes"[^>]*value="500000"|value="500000"[^>]*data-field="max_bytes"/)
    expect(html).toMatch(/placeholder="No limit"[^>]*data-field="max_pixels"|data-field="max_pixels"[^>]*placeholder="No limit"/)
    expect(html).toMatch(
      /data-row="quality.jpeg"><label[^>]*>JPEG quality<\/label><p role="alert">quality must be 1-100/,
    )
  })

  it('shows the built-in defaults as placeholders for an empty entry', () => {
    const html = render(null)
    expect(html).toContain('data-select="Scale to fit"')
    expect(html).toContain('placeholder="80"')
    expect(html.match(/placeholder="No limit"/g)).toHaveLength(2)
    expect(html).not.toMatch(/value="[^"]/)
  })
})

describe('withPath', () => {
  it('sets a top-level key and deletes it when cleared', () => {
    expect(withPath({ height: 100 }, ['width'], 640)).toEqual({ height: 100, width: 640 })
    expect(withPath({ width: 640, height: 100 }, ['width'], undefined)).toEqual({ height: 100 })
  })

  it('sets a quality key and keeps its sibling', () => {
    expect(withPath({ quality: { webp: 70 } }, ['quality', 'jpeg'], 90)).toEqual({ quality: { webp: 70, jpeg: 90 } })
  })

  it('clears a limit, which means no limit', () => {
    expect(withPath({ max_pixels: 1150000, max_bytes: 5000000 }, ['max_bytes'], settingNumber(''))).toEqual({
      max_pixels: 1150000,
    })
  })

  it('removes quality when its last key is cleared', () => {
    expect(withPath({ width: 10, quality: { jpeg: 90 } }, ['quality', 'jpeg'], undefined)).toEqual({ width: 10 })
  })

  it('keeps unknown keys and siblings and never mutates the input', () => {
    const value = { future: { a: 1 }, width: 5, quality: { webp: 70, extra: true } }
    const next = withPath(value, ['quality', 'webp'], undefined)
    expect(next).toEqual({ future: { a: 1 }, width: 5, quality: { extra: true } })
    expect(value.quality).toEqual({ webp: 70, extra: true })
  })

  it('treats a non-object value as empty', () => {
    expect(withPath(null, ['width'], 300)).toEqual({ width: 300 })
    expect(withPath([1, 2], ['quality', 'jpeg'], 50)).toEqual({ quality: { jpeg: 50 } })
    expect(withPath('raw', ['strategy'], undefined)).toEqual({})
  })
})

describe('settingNumber', () => {
  it('reads the whole value, keeping fractions for the schema to reject', () => {
    expect(settingNumber('320')).toBe(320)
    expect(settingNumber('1e3')).toBe(1000)
    expect(settingNumber('85.5')).toBe(85.5)
  })

  it('clears the key for blank or non-numeric input', () => {
    expect(settingNumber('')).toBeUndefined()
    expect(settingNumber('  ')).toBeUndefined()
    expect(settingNumber('abc')).toBeUndefined()
  })
})

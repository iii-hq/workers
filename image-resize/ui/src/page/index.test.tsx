import { formatBytes } from '@iii-dev/console-ui/format'
import type { ComponentProps, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import {
  type FieldErrors,
  initialState,
  MAX_INPUT_BYTES,
  type Options,
  type ResizeResult,
  type State,
  StreamUnavailableError,
} from '../lib/resize'
import { ImageResizeView, type ImageResizeViewProps } from './index'

vi.mock('@iii-dev/console-ui', () => ({
  Button: ({
    children,
    variant,
    size: _size,
    asChild,
    ...props
  }: ComponentProps<'button'> & { variant?: string; size?: string; asChild?: boolean }) =>
    asChild ? (
      children
    ) : (
      <button data-variant={variant} {...props}>
        {children}
      </button>
    ),
  EmptyState: ({ title, description }: { title: string; description: string }) => (
    <div data-empty="">
      {title}|{description}
    </div>
  ),
  Eyebrow: ({ children }: { children?: ReactNode }) => <span>{children}</span>,
  IconButton: ({ label }: { label: string }) => <button type="button" aria-label={label} />,
  ImageThumbnailButton: ({ children, title }: { children?: ReactNode; title: string }) => (
    <button type="button" aria-label={`View ${title}`}>
      {children}
    </button>
  ),
  ImageViewer: () => null,
  Input: ({ onChange: _onChange, ...props }: { onChange?: unknown } & Omit<ComponentProps<'input'>, 'onChange'>) => (
    <input readOnly {...props} />
  ),
  LiveRegion: () => null,
  MetaRow: ({ items }: { items: { label: string; value: string; tone?: string }[] }) => (
    <dl>
      {items.map((item) => (
        <div key={item.label} data-tone={item.tone}>
          <dt>{item.label}</dt>
          <dd>{item.value}</dd>
        </div>
      ))}
    </dl>
  ),
  PageHeader: ({ title }: { title: string }) => <h1>{title}</h1>,
  PageShell: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  Select: ({ value, placeholder }: { value?: string; placeholder?: string }) => (
    <div data-select={value ?? placeholder} />
  ),
  SettingsField: ({
    label,
    description,
    error,
    field,
    renderControl,
  }: {
    label: string
    description?: string
    error?: string
    field: string
    renderControl: (props: { id: string; 'data-field': string }) => ReactNode
  }) => (
    <div data-row={field}>
      {label}
      {description}
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
  Skeleton: () => <span data-skeleton="" />,
  StatusPanel: ({
    variant,
    headline,
    detail,
    action,
  }: {
    variant?: string
    headline: ReactNode
    detail?: ReactNode
    action?: ReactNode
  }) => (
    <div data-panel={variant}>
      <strong>{headline}</strong>
      <p>{detail}</p>
      {action}
    </div>
  ),
}))

const photo = new File([new Uint8Array(1000)], 'photo.jpg', { type: 'image/jpeg' })
const RESULT: ResizeResult = {
  format: 'webp',
  width: 200,
  height: 150,
  source_width: 1600,
  source_height: 1200,
  strategy: 'scale-to-fit',
}
const NO_OPTIONS: Options = { width: '', height: '', quality: '' }
const chosen: State = {
  ...initialState,
  generation: 1,
  source: { file: photo, preview: 'ok', width: 1600, height: 1200 },
}

function render(overrides: Partial<ImageResizeViewProps> = {}) {
  return renderToStaticMarkup(
    <ImageResizeView
      state={initialState}
      urls={{ source: null, result: null }}
      options={NO_OPTIONS}
      errors={{}}
      dragging={false}
      viewer={null}
      announcement={null}
      handlers={{
        onChoose() {},
        onFile() {},
        onDragging() {},
        onRemove() {},
        onOption() {},
        onSubmit() {},
        onViewer() {},
        onPreviewLoad() {},
        onPreviewError() {},
      }}
      {...overrides}
    />,
  )
}

const resizeButton = (html: string) => html.match(/<button[^>]*type="submit"[^>]*>[^<]*<\/button>/)?.[0] ?? ''

describe('ImageResizeView', () => {
  it('starts empty: one focusable choose button, the empty preview and Resize disabled', () => {
    const html = render()
    expect(html).toContain('Drop a JPEG, PNG or WebP image here')
    expect(html).toMatch(/<button[^>]*data-variant="secondary"[^>]*data-autofocus=""[^>]*>Choose an image<\/button>/)
    expect(html.match(/data-variant="secondary"/g)).toHaveLength(1)
    expect(html).not.toContain('data-compact')
    expect(html).toContain('No image yet|Images up to 64 MiB can be resized here and compared with the original.')
    expect(resizeButton(html)).toContain('disabled=""')
    expect(html).toMatch(/<form[^>]*novalidate=""/i)
  })

  it('shows the chosen image as a compact bar and as the original figure', () => {
    const html = render({ state: chosen, urls: { source: 'blob:source', result: null } })
    expect(html).toContain('data-compact="true"')
    expect(html).toContain('photo.jpg')
    expect(html).toContain(formatBytes(1000))
    expect(html).toContain('Choose another')
    expect(html).toContain('aria-label="Remove image"')
    expect(html).toContain('src="blob:source" alt="Original: photo.jpg"')
    expect(html).toContain('1600 × 1200')
    expect(html).toContain('Not resized yet|Set the options and select Resize.')
    expect(html).toContain('1 to 100, for JPEG and WebP.')
    expect(resizeButton(html)).not.toContain('disabled')
  })

  it('says why an image over 64 MiB is refused', () => {
    const size = MAX_INPUT_BYTES + 1
    const html = render({ state: { ...initialState, notice: { kind: 'too-large', name: 'huge.png', size } } })
    expect(html).toContain('data-panel="warn"')
    expect(html).toContain('huge.png is too large')
    expect(html).toContain(`It is ${formatBytes(size)}. image_resize::resize reads images up to 64 MiB.`)
    expect(resizeButton(html)).toContain('disabled=""')
  })

  it('asks for a file when a drop carries none', () => {
    const html = render({ state: { ...chosen, notice: { kind: 'no-file' } } })
    expect(html).toContain('Drop an image file')
    expect(html).toContain(
      'Links and images dragged from a page are not supported. Save the image, then drop the file.',
    )
  })

  it('holds the result frame while the call runs', () => {
    const html = render({ state: { ...chosen, run: { phase: 'running' } } })
    expect(html).toContain('role="status" aria-label="Resizing"')
    expect(html).toContain('data-skeleton=""')
    expect(resizeButton(html)).toMatch(/disabled=""[^>]*>Resizing…</)
  })

  it('shows the result with its size, the change and a download link', () => {
    const blob = new Blob([new Uint8Array(180)], { type: 'image/webp' })
    const state: State = {
      ...chosen,
      source: { file: photo, preview: 'ok' },
      run: { phase: 'done', result: RESULT, blob, preview: 'ok' },
    }
    const html = render({ state, urls: { source: 'blob:source', result: 'blob:result' } })
    expect(html).toContain('src="blob:result" alt="Resized: photo_200x150.webp"')
    expect(html).toContain('200 × 150')
    expect(html).toContain('1600 × 1200')
    expect(html).toContain('<div data-tone="ok"><dt>Change</dt><dd>-82%</dd></div>')
    expect(html).toContain('WebP')
    expect(html).toContain('Scale to fit')
    expect(html).toMatch(/<a[^>]*href="blob:result"[^>]*download="photo_200x150.webp"[^>]*>.*Download<\/a>/)
  })

  it("shows the worker's message when the resize fails", () => {
    const error = { code: 'INVALID_ARGS', message: 'input is not a recognized image; send jpeg, png or webp' }
    const html = render({ state: { ...chosen, run: { phase: 'failed', error } } })
    expect(html).toContain('data-panel="alert"')
    expect(html).toContain('Could not resize photo.jpg')
    expect(html).toContain('input is not a recognized image; send jpeg, png or webp')
    expect(html).toContain('Try again')
  })

  it('explains a stream that did not open', () => {
    const html = render({ state: { ...chosen, run: { phase: 'failed', error: new StreamUnavailableError() } } })
    expect(html).toContain('Could not resize photo.jpg')
    expect(html).toContain(
      'The image stream did not open or was cut off. If this happens with every image, the ADE serving this page is older than the page and does not relay streams yet: update the ade worker and reload.',
    )
  })

  it('keeps Resize available when the browser cannot preview the file', () => {
    const file = new File([new Uint8Array(10)], 'scan.png', { type: 'image/png' })
    const html = render({
      state: { ...initialState, generation: 1, source: { file, preview: 'failed' } },
      urls: { source: 'blob:source', result: null },
    })
    expect(html).toContain('Preview unavailable')
    expect(html).toContain('The browser could not display this file.')
    expect(html).not.toContain('<img')
    expect(html).not.toContain('Dimensions')
    expect(html).toContain('PNG is lossless; quality does not apply.')
    expect(resizeButton(html)).not.toContain('disabled')
  })

  it('shows each invalid option next to its field', () => {
    const errors: FieldErrors = {
      width: 'Enter a whole number of pixels, 1 or more.',
      quality: 'Enter a quality from 1 to 100.',
    }
    const html = render({ state: chosen, options: { ...NO_OPTIONS, width: '0', quality: '101' }, errors })
    expect(html).toMatch(/data-row="width">Width<p role="alert">Enter a whole number of pixels, 1 or more.<\/p>/)
    expect(html).toMatch(/data-row="quality">Quality.*<p role="alert">Enter a quality from 1 to 100.<\/p>/)
    expect(html).not.toMatch(/data-row="height">Height<p role="alert">/)
  })
})

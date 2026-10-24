// biome-ignore-all lint/suspicious/noTemplateCurlyInString: these literals intentionally exercise ${VAR} configuration templates.
import type { Host, JsonValue } from '@iii-dev/console-ui'
import type { ComponentProps, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { CredentialField, type CredentialFieldProps } from './CredentialField'
import { StorageConfigForm } from './index'
import { TestBucketRow } from './TestBucketRow'

vi.mock('@iii-dev/console-ui', () => ({
  Button: ({
    children,
    variant: _variant,
    size: _size,
    ...props
  }: ComponentProps<'button'> & { variant?: string; size?: string }) => <button {...props}>{children}</button>,
  Chip: ({ children, tone: _tone, ...props }: ComponentProps<'span'> & { tone?: string }) => (
    <span {...props}>{children}</span>
  ),
  Checkbox: ({ label, ...props }: ComponentProps<'input'> & { label?: ReactNode }) => (
    <label>
      <input type="checkbox" readOnly {...props} />
      {label}
    </label>
  ),
  Eyebrow: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  IconButton: ({ children, label }: { children?: ReactNode; label: string }) => (
    <button type="button" aria-label={label}>
      {children}
    </button>
  ),
  Input: ({
    onChange: _onChange,
    ...props
  }: { onChange?: (next: string) => void } & Omit<ComponentProps<'input'>, 'onChange'>) => (
    <input readOnly {...props} />
  ),
  List: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  ListItem: ({
    leading,
    label,
    description,
    trailing,
    selected: _selected,
    ...props
  }: ComponentProps<'button'> & {
    leading?: ReactNode
    label?: ReactNode
    description?: ReactNode
    trailing?: ReactNode
    selected?: boolean
  }) => (
    <button type="button" {...props}>
      {leading}
      <span data-label="">{label}</span>
      <span data-description="">{description}</span>
      {trailing}
    </button>
  ),
  Select: ({ value }: { value?: string }) => <div data-select={value} />,
  StatusDot: (props: ComponentProps<'span'> & { tone?: string }) => <span data-status-dot={props.tone} />,
  StatusPanel: ({ headline, detail }: { headline: ReactNode; detail?: ReactNode }) => (
    <div role="status">
      {headline}
      {detail}
    </div>
  ),
  useConfirm: () => ({ confirm: async () => true, dialog: null }),
}))

vi.mock('@iii-dev/console-ui/hooks', () => ({
  useContainerNarrow: () => ({ ref: () => {}, narrow: false }),
}))

const host = {
  iii: { trigger: async () => null },
} as unknown as Host

function field(overrides: Partial<CredentialFieldProps> & { value: JsonValue | undefined }) {
  return renderToStaticMarkup(
    <CredentialField
      host={host}
      id="storage-cfg-photos-secret_access_key"
      path="buckets/photos/secret_access_key"
      label="Secret access key"
      target="STORAGE_PHOTOS_SECRET_ACCESS_KEY"
      bucket="photos"
      onChange={() => {}}
      {...overrides}
    />,
  )
}

describe('credential fields', () => {
  it('shows a reference and whether the store has it, never a value', () => {
    const html = field({ value: 'secret://STORAGE_PHOTOS_SECRET_ACCESS_KEY' })
    expect(html).toContain('secret://STORAGE_PHOTOS_SECRET_ACCESS_KEY')
    expect(html).toContain('Checking the secrets store…')
    expect(html).toContain('Replace')
    expect(html).toContain('Remove')
    expect(html).not.toContain('<input')
  })

  it('shows env:// and ${VAR} as what they are', () => {
    expect(field({ value: 'env://AWS_SECRET_ACCESS_KEY' })).toContain('A variable of the secrets worker’s env file')
    const template = field({ value: '${AWS_SECRET_ACCESS_KEY}' })
    expect(template).toContain('${AWS_SECRET_ACCESS_KEY}')
    expect(template).toContain('Expanded from the engine’s environment')
  })

  it('flags a plain-text secret without printing it', () => {
    const html = field({ value: 'wJalrXUtnFEMI-K7MDENG' })
    expect(html).toContain('Plain text')
    expect(html).toContain('Saved in configuration, which is meant to be shared.')
    expect(html).not.toContain('wJalrXUtnFEMI')
  })

  it('shows an identifier kept in configuration', () => {
    const html = field({
      value: 'minioadmin',
      mode: 'identifier',
      label: 'Access key ID',
      target: 'STORAGE_PHOTOS_ACCESS_KEY_ID',
    })
    expect(html).toContain('minioadmin')
    expect(html).toContain('Kept in configuration')
    expect(html).toContain('Edit')
  })

  it('offers to store an empty field under the bucket’s name', () => {
    const html = field({ value: undefined })
    expect(html).toContain('type="password"')
    expect(html).toContain('Stored as secret://STORAGE_PHOTOS_SECRET_ACCESS_KEY')
    expect(html).toContain('Save to secrets')
    expect(html).not.toContain('Keep in configuration')
  })

  it('makes keeping an account ID in configuration the default', () => {
    const html = field({
      value: undefined,
      mode: 'identifier',
      preferText: true,
      label: 'Account ID',
      target: 'STORAGE_CDN_ACCOUNT_ID',
    })
    expect(html).toContain('type="text"')
    expect(html).toContain('Kept in configuration')
    expect(html).toContain('Keep in configuration')
    expect(html).toContain('Save to secrets')
  })

  it('explains a worker that predates references and keeps values inline', () => {
    const html = field({ value: undefined, referencesSupported: false })
    expect(html).toContain('iii trigger compose::update worker=storage')
    expect(html).toContain('Plain text in configuration')
    expect(html).toContain('Keep as plain text')
    expect(html).not.toContain('Save to secrets')
  })

  it('takes a service-account key as a pasted or uploaded JSON', () => {
    const html = field({
      value: undefined,
      mode: 'json',
      label: 'Service-account key',
      target: 'STORAGE_DOCS_CREDENTIALS_JSON',
    })
    expect(html).toContain('<textarea')
    expect(html).toContain('Upload file')
    expect(html).toContain('Stored as secret://STORAGE_DOCS_CREDENTIALS_JSON')
  })

  it('keeps a stored value the worker hides', () => {
    const html = field({ value: '__redacted__' })
    expect(html).toContain('A stored value the worker keeps hidden')
    expect(html).toContain('Replace')
    expect(html).not.toContain('Remove')
  })
})

describe('Test bucket', () => {
  it('says what blocks it', () => {
    const html = renderToStaticMarkup(
      <TestBucketRow state={undefined} blocker="Fill in the region to test" onTest={() => {}} />,
    )
    expect(html).toContain('disabled=""')
    expect(html).toContain('Fill in the region to test')
  })

  it('shows the staged result and its hint', () => {
    const html = renderToStaticMarkup(
      <TestBucketRow
        state={{
          status: 'done',
          ok: false,
          text: 'Bucket failed · NoSuchBucket',
          hint: 'There is no bucket “photos” at this endpoint yet',
        }}
        blocker={null}
        onTest={() => {}}
      />,
    )
    expect(html).toContain('storage-cfg-test-fail')
    expect(html).toContain('Bucket failed · NoSuchBucket')
    expect(html).toContain('There is no bucket “photos” at this endpoint yet')
  })

  it('explains a worker without storage::testBucket', () => {
    const html = renderToStaticMarkup(
      <TestBucketRow state={{ status: 'unsupported' }} blocker={null} onTest={() => {}} />,
    )
    expect(html).toContain('storage::testBucket')
    expect(html).toContain('iii trigger compose::update worker=storage')
  })
})

describe('the form', () => {
  it('lists buckets with their icon colour and provider', () => {
    const html = renderToStaticMarkup(
      <StorageConfigForm
        id="storage"
        schema={null}
        host={host}
        value={{
          buckets: {
            photos: {
              provider: 's3',
              region: 'us-east-1',
              endpoint_url: 'http://minio:9000',
              display: { icon: 'cloud', color: '#2563EB' },
            },
            docs: { provider: 'gcs' },
            scratch: { provider: 'local' },
          },
        }}
        onChange={() => {}}
      />,
    )
    expect(html).toContain('S3-compatible')
    expect(html).toContain('GCS')
    expect(html).toContain('Local')
    expect(html).toContain('color:#2563EB')
    expect(html).toContain('lucide-cloud')
  })
})

import { describe, expect, it } from 'vitest'
import { cloudRecipient, cloudTextError, templateComponents, templateText } from '../src/cloud-model'
import { isOpaqueCloudField, withCloudField, withCloudValue } from '../src/model'

describe('Cloud API addressing stays separate from native JIDs', () => {
  it.each([
    ['5511999999999', '+5511999999999'],
    [' +55 (11) 99999-9999 ', '+5511999999999'],
    ['US.abc123XYZ', 'US.abc123XYZ'],
    ['BR.ENT.Abc123', 'BR.ENT.Abc123'],
    [`BR.${'a'.repeat(128)}`, `BR.${'a'.repeat(128)}`],
  ])('normalizes %s without changing business-scoped identities', (input, expected) => {
    expect(cloudRecipient(input)).toBe(expected)
  })
  it.each(['', '123456', '0123456789', '+1234567890123456', '1234567@s.whatsapp.net', '12345@lid', '120000@g.us', 'cloud:+12345678', 'br.abc', 'BR.ent.abc', 'BR.abc-def', `BR.${'a'.repeat(129)}`])('rejects unsupported recipient %s before sending', (input) => {
    expect(() => cloudRecipient(input)).toThrow()
  })
  it('measures the text limit in Unicode characters without altering the draft', () => {
    expect(cloudTextError('  \n')).toBeTruthy()
    expect(cloudTextError('😀'.repeat(4096))).toBeNull()
    expect(cloudTextError('😀'.repeat(4097))).toContain('4,096')
  })
})

describe('template parameters remain explicit', () => {
  it('omits blank parameters and preserves caller-supplied component data', () => {
    expect(templateComponents('  ')).toBeUndefined()
    const components = [{ type: 'body', parameters: [{ type: 'text', parameter_name: 'customer', text: 'Ada' }] }]
    expect(templateComponents(JSON.stringify(components))).toEqual(components)
  })
  it.each(['{', '{}', 'null', '[1]', '["body"]', '[null]'])('rejects invalid components %s', (value) => {
    expect(() => templateComponents(value)).toThrow()
  })
  it('bounds pasted parameter JSON and only previews template text', () => {
    expect(() => templateComponents(JSON.stringify([{ text: 'x'.repeat(65536) }]))).toThrow('64 KiB')
    expect(templateText({ id: '1', name: 'update', language: 'en_US', status: 'APPROVED', category: 'UTILITY', components: [{ type: 'BODY', text: 'Hello {{1}}' }, { type: 'BUTTONS', buttons: [{ text: 'Open' }] }] })).toBe('Hello {{1}}')
  })
})

describe('nested Cloud configuration preserves host-owned raw values', () => {
  it('edits only the requested child, retaining environment tokens and unknown siblings', () => {
    const original = { cloud: { access_token: '${META_TOKEN}', future: { nested: true } }, data_dir: '${DATA}', other: 4 }
    expect(withCloudField(original, 'phone_number_id', '123')).toEqual({ ...original, cloud: { ...original.cloud, phone_number_id: '123' } })
    expect(original.cloud).not.toHaveProperty('phone_number_id')
    expect(withCloudField(original, 'access_token', undefined)).toEqual({ ...original, cloud: { future: { nested: true } } })
  })
  it.each(['${CLOUD}', null, [1], 3])('keeps an opaque Cloud block until explicit replacement: %s', (cloud) => {
    const original = { cloud, future: true }
    expect(withCloudField(original, 'enabled', true)).toBe(original)
    expect(withCloudValue(original, {})).toEqual({ cloud: {}, future: true })
  })
  it('does not replace an opaque root or materialize defaults on unrelated edits', () => {
    expect(withCloudField('${CONFIG}', 'access_token', 'secret')).toBe('${CONFIG}')
    expect(withCloudValue('${CONFIG}', {})).toBe('${CONFIG}')
    expect(withCloudField({ future: 1 }, 'enabled', true)).toEqual({ future: 1, cloud: { enabled: true } })
  })
  it('recognizes environment references, raw booleans, nulls and custom credentials', () => {
    expect(isOpaqueCloudField('enabled', '${CLOUD_ENABLED}')).toBe(true)
    expect(isOpaqueCloudField('access_token', '${META_TOKEN}')).toBe(true)
    expect(isOpaqueCloudField('access_token', { provider: 'vault' })).toBe(true)
    expect(isOpaqueCloudField('enabled', null)).toBe(true)
    expect(isOpaqueCloudField('enabled', false)).toBe(false)
    expect(isOpaqueCloudField('api_version', undefined)).toBe(false)
  })
})

import { describe, expect, it } from 'vitest'
import { cloudRecipient, cloudTextError, templateComponents, templateInputs, templateInputComponents, templateSendComponents, templateStarterComponents, templateText, type CloudTemplate } from '../src/cloud-model'
import { isOpaqueCloudField, withCloudField, withCloudValue } from '../src/model'

describe('Cloud API addressing stays separate from native JIDs', () => {
  it('previews only entered text parameters without changing or recursively substituting the approved text', () => {
    const selected: CloudTemplate = { id: '1', name: 'preview', language: 'en_US', status: 'APPROVED', category: 'UTILITY', components: [{ type: 'HEADER', text: 'Report {{1}}' }, { type: 'BODY', text: 'Hello {{ customer }}: {{1}} / {{2}}' }] }
    expect(templateText(selected)).toBe('Report {{1}}\n\nHello {{ customer }}: {{1}} / {{2}}')
    expect(templateText(selected, { 'header:1': ' October ', 'body:customer': 'Ada', 'body:1': '$& {{2}}' })).toBe('Report October\n\nHello Ada: $& {{2}} / {{2}}')
    expect(selected.components[0]).toEqual({ type: 'HEADER', text: 'Report {{1}}' })
  })

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
  it.each(['{', '{}', 'null', '[1]', '["body"]', '[null]', '[{}]', '[{"type":"footer","parameters":[]}]', '[{"type":"body"}]', '[{"type":"body","parameters":[]}]', '[{"type":"body","parameters":[{}]}]', '[{"type":"body","parameters":[{"type":"text","text":" "}]}]', '[{"type":"header","parameters":[{"type":"image","image":{}}]}]', '[{"type":"button","parameters":[{"type":"text","text":"suffix"}]}]', '[{"type":"button","sub_type":"url","index":"x","parameters":[{"type":"text","text":"suffix"}]}]'])('rejects invalid components %s', (value) => {
    expect(() => templateComponents(value)).toThrow()
  })

  it('preserves complete media, button and advanced payloads rather than rewriting them', () => {
    const components = [
      { type: 'header', parameters: [{ type: 'document', document: { link: 'https://example.com/report.pdf', filename: 'report.pdf' } }] },
      { type: 'body', parameters: [{ type: 'date_time', date_time: { fallback_value: 'October 7' } }, { type: 'group_id', group_id: 'GroupOne' }] },
      { type: 'button', sub_type: 'url', index: '0', parameters: [{ type: 'text', text: 'order-123' }] },
      { type: 'button', sub_type: 'copy_code', index: 1, parameters: [{ type: 'coupon_code', coupon_code: 'OCTOBER' }] },
      { type: 'button', sub_type: 'flow', index: '2', parameters: [{ type: 'action', action: { flow_token: 'token', flow_action_data: { screen: 'START' } } }] },
      { type: 'button', sub_type: 'future_button', index: '3', parameters: [{ type: 'future_parameter', future_parameter: { actual_value: 'value' } }] },
      { type: 'carousel', cards: [{ card_index: 0, components: [{ type: 'header', parameters: [{ type: 'product', product: { catalog_id: '123', product_retailer_id: 'sku' } }] }] }] },
      { type: 'limited_time_offer', parameters: [{ type: 'limited_time_offer', limited_time_offer: { expiration_time_ms: '1791417600000' } }] },
    ]
    expect(templateComponents(JSON.stringify(components))).toEqual(components)
  })

  it.each([
    { type: 'currency', currency: { code: 'USD', amount_1000: 12000 } },
    { type: 'date_time', date_time: {} },
    { type: 'payload', payload: '' },
    { type: 'action', action: {} },
    { type: 'future_parameter' },
  ])('rejects incomplete typed parameters %j', (parameter) => {
    expect(() => templateComponents(JSON.stringify([{ type: 'body', parameters: [parameter] }]))).toThrow()
  })

  it('validates nested carousel components', () => {
    expect(() => templateComponents(JSON.stringify([{ type: 'carousel', cards: [{ card_index: 0, components: [{}] }] }]))).toThrow()
  })
  it('bounds pasted parameter JSON and only previews template text', () => {
    expect(() => templateComponents(JSON.stringify([{ text: 'x'.repeat(65536) }]))).toThrow('64 KiB')
    expect(templateText({ id: '1', name: 'update', language: 'en_US', status: 'APPROVED', category: 'UTILITY', components: [{ type: 'BODY', text: 'Hello {{1}}' }, { type: 'BUTTONS', buttons: [{ text: 'Open' }] }] })).toBe('Hello {{1}}')
  })
})

describe('guided template parameters', () => {
  const template = (components: CloudTemplate['components'], parameter_format = 'POSITIONAL'): CloudTemplate => ({ id: '1', name: 'update', language: 'en_US', status: 'APPROVED', category: 'UTILITY', components, parameter_format })

  it('serializes positional header and body fields in numerical order, deduplicating repeated placeholders', () => {
    const selected = template([{ type: 'HEADER', format: 'TEXT', text: 'Order {{1}}' }, { type: 'BODY', text: '{{2}}: Hello {{1}}. Repeat {{1}}.' }])
    expect(templateInputs(selected)).toEqual({ advancedRequired: false, fields: [
      { key: 'header:1', component: 'header', parameter: '1', label: 'Header parameter 1 — {{1}}' },
      { key: 'body:1', component: 'body', parameter: '1', label: 'Body parameter 1 — {{1}}' },
      { key: 'body:2', component: 'body', parameter: '2', label: 'Body parameter 2 — {{2}}' },
    ] })
    expect(templateInputComponents(selected, { 'header:1': '456', 'body:1': 'Ada', 'body:2': 'Today' })).toEqual([
      { type: 'header', parameters: [{ type: 'text', text: '456' }] },
      { type: 'body', parameters: [{ type: 'text', text: 'Ada' }, { type: 'text', text: 'Today' }] },
    ])
    expect(() => templateInputComponents(selected, { 'header:1': '456', 'body:1': 'Ada', 'body:2': ' ' })).toThrow('Body parameter 2 — {{2}}')
  })

  it('sends names for named text header and body parameters', () => {
    const selected = template([{ type: 'HEADER', format: 'TEXT', text: 'For {{customer}}' }, { type: 'BODY', text: 'Order {{order_id}} for {{customer}}' }], 'NAMED')
    expect(templateInputComponents(selected, { 'header:customer': 'Ada', 'body:order_id': '123', 'body:customer': 'Ada' })).toEqual([
      { type: 'header', parameters: [{ type: 'text', parameter_name: 'customer', text: 'Ada' }] },
      { type: 'body', parameters: [{ type: 'text', parameter_name: 'order_id', text: '123' }, { type: 'text', parameter_name: 'customer', text: 'Ada' }] },
    ])
  })

  it('does not invent send parameters for static text, footer or static buttons', () => {
    const selected = template([{ type: 'BODY', text: 'Your request is ready.' }, { type: 'FOOTER', text: 'Support team' }, { type: 'BUTTONS', buttons: [{ type: 'URL', text: 'Open', url: 'https://example.com' }, { type: 'PHONE_NUMBER', text: 'Call', phone_number: '+15551234567' }, { type: 'QUICK_REPLY', text: 'Thanks' }] }])
    expect(templateInputs(selected)).toEqual({ advancedRequired: false, fields: [] })
    expect(templateInputComponents(selected, {})).toBeUndefined()
  })

  it('uses the same guided and explicit parameter validation before enabling and sending', () => {
    const selected = template([{ type: 'BODY', text: 'Hello {{1}}' }])
    expect(() => templateSendComponents(selected, {}, '')).toThrow('Body parameter 1')
    expect(templateSendComponents(selected, { 'body:1': 'Ada' }, '')).toEqual([{ type: 'body', parameters: [{ type: 'text', text: 'Ada' }] }])
    expect(() => templateSendComponents(selected, {}, '{')).toThrow('valid JSON')
    expect(() => templateSendComponents(selected, {}, '[]')).toThrow('required component objects')
    const explicit = [{ type: 'body', parameters: [{ type: 'currency', currency: { fallback_value: '$12.00', code: 'USD', amount_1000: 12000 } }] }]
    expect(templateSendComponents(selected, {}, JSON.stringify(explicit))).toEqual(explicit)
    expect(templateSendComponents(template([{ type: 'BODY', text: 'Hello' }]), {}, '[]')).toEqual([])
  })

  it('requires nonempty explicit components when the template cannot use guided text fields', () => {
    const selected = template([{ type: 'HEADER', format: 'IMAGE' }])
    expect(() => templateSendComponents(selected, {}, '')).toThrow('Advanced JSON')
    expect(() => templateSendComponents(selected, {}, '[]')).toThrow('required component objects')
    const explicit = [{ type: 'header', parameters: [{ type: 'image', image: { id: '12345' } }] }]
    expect(templateSendComponents(selected, {}, JSON.stringify(explicit))).toEqual(explicit)
  })

  it('requires every known header and positional body placeholder in an explicit override', () => {
    const selected = template([{ type: 'HEADER', format: 'TEXT', text: 'Order {{1}}' }, { type: 'BODY', text: 'Hello {{1}}: {{2}}. Repeat {{1}}.' }])
    const header = { type: 'header', parameters: [{ type: 'text', text: '123' }] }
    const body = { type: 'body', parameters: [{ type: 'text', text: 'Ada' }, { type: 'currency', currency: { fallback_value: '$12.00', code: 'USD', amount_1000: 12000 } }] }
    expect(() => templateSendComponents(selected, {}, JSON.stringify([body]))).toThrow('header')
    expect(() => templateSendComponents(selected, {}, JSON.stringify([header, { ...body, parameters: body.parameters.slice(0, 1) }]))).toThrow('body')
    expect(templateSendComponents(selected, {}, JSON.stringify([header, body]))).toEqual([header, body])
  })

  it('requires the selected named placeholders rather than accepting unrelated or unnamed parameters', () => {
    const selected = template([{ type: 'BODY', text: 'For {{customer}}, order {{order_id}}' }], 'NAMED')
    const customer = { type: 'text', parameter_name: 'customer', text: 'Ada' }
    const order = { type: 'text', parameter_name: 'order_id', text: '123' }
    expect(() => templateSendComponents(selected, {}, JSON.stringify([{ type: 'body', parameters: [customer, { ...order, parameter_name: 'unrelated' }] }]))).toThrow('order_id')
    expect(() => templateSendComponents(selected, {}, JSON.stringify([{ type: 'body', parameters: [{ type: 'text', text: 'Ada' }, order] }]))).toThrow('customer')
    expect(templateSendComponents(selected, {}, JSON.stringify([{ type: 'body', parameters: [order, customer] }]))).toEqual([{ type: 'body', parameters: [order, customer] }])
  })

  it('requires known media headers and dynamic buttons alongside explicit body parameters', () => {
    const selected = template([{ type: 'HEADER', format: 'IMAGE' }, { type: 'BODY', text: 'Hello {{1}}' }, { type: 'BUTTONS', buttons: [{ type: 'URL', url: 'https://example.com/{{1}}', text: 'Open' }] }])
    const body = { type: 'body', parameters: [{ type: 'text', text: 'Ada' }] }
    const header = { type: 'header', parameters: [{ type: 'image', image: { id: '123' } }] }
    const button = { type: 'button', sub_type: 'url', index: '0', parameters: [{ type: 'text', text: 'order-123' }] }
    expect(() => templateSendComponents(selected, {}, JSON.stringify([body, button]))).toThrow('image')
    expect(() => templateSendComponents(selected, {}, JSON.stringify([body, header]))).toThrow('button')
    expect(templateSendComponents(selected, {}, JSON.stringify([body, header, button]))).toEqual([body, header, button])
  })

  it('accepts explicit authentication body and OTP button parameters', () => {
    const selected = { ...template([{ type: 'BODY', text: '{{1}} is your verification code.' }, { type: 'BUTTONS', buttons: [{ type: 'OTP', otp_type: 'COPY_CODE', text: 'Copy code' }] }]), category: 'AUTHENTICATION' }
    const components = [{ type: 'body', parameters: [{ type: 'text', text: '123456' }] }, { type: 'button', sub_type: 'url', index: '0', parameters: [{ type: 'text', text: '123456' }] }]
    expect(templateSendComponents(selected, {}, JSON.stringify(components))).toEqual(components)
  })

  it('derives editable starter components without supplying text or media values', () => {
    const selected = template([{ type: 'HEADER', format: 'DOCUMENT' }, { type: 'BODY', text: 'Hello {{customer}}' }, { type: 'BUTTONS', buttons: [{ type: 'URL', url: 'https://example.com/{{1}}', text: 'Open' }, { type: 'COPY_CODE', text: 'Copy' }] }], 'NAMED')
    const starter = templateStarterComponents(selected)
    expect(JSON.parse(starter)).toEqual([
      { type: 'header', parameters: [{ type: 'document', document: { id: '' } }] },
      { type: 'body', parameters: [{ type: 'text', parameter_name: 'customer', text: '' }] },
      { type: 'button', sub_type: 'url', index: '0', parameters: [{ type: 'text', text: '' }] },
      { type: 'button', sub_type: 'copy_code', index: '1', parameters: [{ type: 'coupon_code', coupon_code: '' }] },
    ])
    expect(() => templateSendComponents(selected, {}, starter)).toThrow()
    expect(templateStarterComponents(template([{ type: 'CAROUSEL', cards: [] }]))).toBe('')
  })

  it.each([
    [{ type: 'HEADER', format: 'IMAGE' }],
    [{ type: 'BUTTONS', buttons: [{ type: 'URL', url: 'https://example.com/{{1}}', text: 'Open' }] }],
    [{ type: 'BUTTONS', buttons: [{ type: 'COPY_CODE', text: 'Copy offer code' }] }],
    [{ type: 'CAROUSEL', cards: [] }],
    [{ type: 'BODY', text: 'Missing index {{2}}' }],
  ])('requires explicit JSON for unsupported component structure %j', (component) => {
    const selected = template([component])
    expect(templateInputs(selected).advancedRequired).toBe(true)
    expect(() => templateInputComponents(selected, {})).toThrow('Advanced JSON')
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

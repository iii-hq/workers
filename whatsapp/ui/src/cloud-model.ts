import type { JsonValue } from '@iii-dev/console-ui'
import { isObject } from './model'

export interface CloudStatus {
  enabled: boolean
  configured: boolean
  missing_fields: string[]
  api_version: string
  business_account_id: string
  phone_number_id: string
  access_token_set: boolean
  app_secret_set: boolean
  verify_token_set: boolean
  webhook_path: string
  webhook_ready: boolean
}
export interface CloudTunnelStatus {
  enabled: boolean
  status: 'stopped' | 'starting' | 'ready' | 'reconnecting' | 'failed' | 'unavailable'
  tunnel_id: string
  public_url: string | null
  webhook_url: string | null
  webhook_path: string
  error: string | null
}
export interface CloudConnection {
  phone: { id: string; display_phone_number?: string; verified_name?: string; quality_rating?: string; code_verification_status?: string }
  phone_in_business_account: boolean
  subscribed_apps: { id: string; name?: string }[]
}
export interface CloudTemplate {
  id: string
  name: string
  language: string
  status: string
  category: string
  components: JsonValue[]
  parameter_format?: string
}
export interface TemplatePage { templates: CloudTemplate[]; next_cursor: string | null }
export interface CloudSent { message_id: string; chat_id: string; status: 'accepted'; message_status?: string }

export function cloudRecipient(raw: string): string {
  const value = raw.trim()
  const phone = value.replace(/[ ()-]/g, '')
  if (/^\+?[1-9][0-9]{6,14}$/.test(phone)) return `+${phone.replace(/^\+/, '')}`
  if (/^[A-Z]{2}\.(?:ENT\.)?[A-Za-z0-9]{1,128}$/.test(value)) return value
  throw new Error('Enter an international phone number with 7–15 digits or a business-scoped user ID. Native WhatsApp JIDs are not supported here.')
}

export function cloudTextError(value: string): string | null {
  if (!value.trim()) return 'Write a message.'
  return Array.from(value).length > 4096 ? 'Text messages can contain up to 4,096 characters.' : null
}

const nonemptyString = (value: unknown): value is string => typeof value === 'string' && !!value.trim()
const componentIndex = (value: unknown): boolean => typeof value === 'number' ? Number.isSafeInteger(value) && value >= 0 : typeof value === 'string' && /^\d+$/.test(value)

function validateTemplateParameter(value: unknown): void {
  if (!isObject(value) || !nonemptyString(value.type)) throw new Error('Each template parameter needs a type and its value.')
  const payload = value[value.type]
  let valid: boolean
  switch (value.type) {
    case 'text': case 'payload': case 'coupon_code': case 'group_id':
      valid = nonemptyString(payload)
      break
    case 'image': case 'video': case 'document':
      valid = isObject(payload) && (nonemptyString(payload.id) || nonemptyString(payload.link))
      break
    case 'currency':
      valid = isObject(payload) && nonemptyString(payload.fallback_value) && nonemptyString(payload.code) && typeof payload.amount_1000 === 'number' && Number.isFinite(payload.amount_1000)
      break
    case 'date_time':
      valid = isObject(payload) && nonemptyString(payload.fallback_value)
      break
    case 'location':
      valid = isObject(payload) && [payload.latitude, payload.longitude].every((coordinate) => (typeof coordinate === 'number' || nonemptyString(coordinate)) && Number.isFinite(Number(coordinate)))
      break
    case 'product':
      valid = isObject(payload) && nonemptyString(payload.catalog_id) && nonemptyString(payload.product_retailer_id)
      break
    case 'limited_time_offer':
      valid = isObject(payload) && componentIndex(payload.expiration_time_ms)
      break
    default:
      // The worker passes advanced Meta payloads through, including future parameter types.
      valid = nonemptyString(payload) || isObject(payload) && Object.keys(payload).length > 0
  }
  if (!valid) throw new Error(`The ${value.type} template parameter needs a complete ${value.type} value${['image', 'video', 'document'].includes(value.type) ? ' with a media id or link' : ''}.`)
  if (value.parameter_name !== undefined && !nonemptyString(value.parameter_name)) throw new Error('A named template parameter needs a nonempty parameter_name.')
}

function validateTemplateComponent(value: unknown): void {
  if (!isObject(value) || typeof value.type !== 'string' || !['header', 'body', 'button', 'carousel', 'limited_time_offer'].includes(value.type)) {
    throw new Error('Each template component needs a send type: header, body, button, carousel or limited_time_offer.')
  }
  if (value.type === 'carousel') {
    if (!Array.isArray(value.cards) || !value.cards.length) throw new Error('A carousel template component needs cards with card_index and components.')
    for (const card of value.cards) {
      if (!isObject(card) || !componentIndex(card.card_index) || !Array.isArray(card.components) || !card.components.length) throw new Error('Each carousel card needs a card_index and component objects.')
      card.components.forEach(validateTemplateComponent)
    }
    return
  }
  if (value.type === 'button' && (!nonemptyString(value.sub_type) || !componentIndex(value.index))) throw new Error('Each button template component needs a sub_type and a numeric index.')
  if (!Array.isArray(value.parameters) || !value.parameters.length) throw new Error(`The ${value.type} template component needs a nonempty parameters array.`)
  value.parameters.forEach(validateTemplateParameter)
}

export function templateComponents(raw: string): JsonValue[] | undefined {
  if (!raw.trim()) return undefined
  if (new TextEncoder().encode(raw).length > 65536) throw new Error('Template parameters exceed the 64 KiB limit.')
  let value: unknown
  try { value = JSON.parse(raw) } catch { throw new Error('Template parameters must be valid JSON.') }
  if (!Array.isArray(value) || !value.every(isObject)) throw new Error('Template parameters must be a JSON array of component objects.')
  value.forEach(validateTemplateComponent)
  return value
}

export function templateText(template: CloudTemplate, values?: Record<string, string>): string {
  return template.components.flatMap((part) => {
    if (!isObject(part) || typeof part.text !== 'string') return []
    const component = typeof part.type === 'string' ? part.type.toLowerCase() : ''
    return [values ? part.text.replace(/{{\s*([^{}]+?)\s*}}/g, (placeholder, parameter: string) => values[`${component}:${parameter.trim()}`]?.trim() || placeholder) : part.text]
  }).join('\n\n')
}

interface TemplateInput {
  key: string
  component: 'header' | 'body'
  parameter: string
  label: string
}

/** Only infer text parameters. Other component types remain an explicit JSON choice. */
export function templateInputs(template: CloudTemplate): { fields: TemplateInput[]; advancedRequired: boolean } {
  const fields: TemplateInput[] = []
  const named = template.parameter_format?.toUpperCase() === 'NAMED'
  let advancedRequired = template.category === 'AUTHENTICATION'
    || !!template.parameter_format && !['NAMED', 'POSITIONAL'].includes(template.parameter_format.toUpperCase())
  for (const part of template.components) {
    if (!isObject(part)) { advancedRequired = true; continue }
    if (part.type === 'FOOTER') continue
    if (part.type === 'BUTTONS') {
      if (!Array.isArray(part.buttons) || !part.buttons.every((button) => isObject(button) && (
        button.type === 'QUICK_REPLY' || button.type === 'PHONE_NUMBER'
        || button.type === 'URL' && typeof button.url === 'string' && !button.url.includes('{{')
      ))) advancedRequired = true
      continue
    }
    if ((part.type !== 'BODY' && part.type !== 'HEADER') || (part.type === 'HEADER' && part.format !== 'TEXT') || typeof part.text !== 'string') {
      advancedRequired = true
      continue
    }
    const placeholders = [...part.text.matchAll(/{{\s*([^{}]+?)\s*}}/g)].map((match) => match[1].trim())
    const parameters = [...new Set(placeholders)]
    if (!named) parameters.sort((a, b) => Number(a) - Number(b))
    if (parameters.some((parameter, index) => named ? !/^[a-z][a-z0-9_]*$/.test(parameter) : parameter !== String(index + 1))
      || (part.type === 'HEADER' && parameters.length > 1)) advancedRequired = true
    const component = part.type === 'HEADER' ? 'header' : 'body'
    for (const parameter of parameters) fields.push({ key: `${component}:${parameter}`, component, parameter, label: `${component === 'header' ? 'Header' : 'Body'} parameter ${parameter} — {{${parameter}}}` })
  }
  return { fields, advancedRequired }
}

export function templateInputComponents(template: CloudTemplate, values: Record<string, string>): JsonValue[] | undefined {
  const { fields, advancedRequired } = templateInputs(template)
  if (advancedRequired) throw new Error('This template needs Advanced JSON components before sending.')
  const components: JsonValue[] = []
  for (const component of ['header', 'body'] as const) {
    const parameters = fields.filter((field) => field.component === component).map((field): JsonValue => {
      const text = values[field.key] ?? ''
      if (!text.trim()) throw new Error(`Enter ${field.label} before sending.`)
      if (template.parameter_format?.toUpperCase() === 'NAMED') return { type: 'text', parameter_name: field.parameter, text }
      return { type: 'text', text }
    })
    if (parameters.length) components.push({ type: component, parameters })
  }
  return components.length ? components : undefined
}

/** A read-only starting shape. Empty values must be replaced with the caller's actual send values. */
export function templateStarterComponents(template: CloudTemplate): string {
  const components: JsonValue[] = []
  const { fields } = templateInputs(template)
  const named = template.parameter_format?.toUpperCase() === 'NAMED'
  for (const type of ['header', 'body'] as const) {
    const parameters = fields.filter((field) => field.component === type)
    if (parameters.length && parameters.every((field, index) => named ? /^[a-z][a-z0-9_]*$/.test(field.parameter) : field.parameter === String(index + 1))) {
      components.push({ type, parameters: parameters.map((field): JsonValue => named ? { type: 'text', parameter_name: field.parameter, text: '' } : { type: 'text', text: '' }) })
    } else if (type === 'header') {
      const header = template.components.find((part) => isObject(part) && part.type === 'HEADER')
      if (isObject(header) && typeof header.format === 'string' && ['IMAGE', 'VIDEO', 'DOCUMENT'].includes(header.format)) {
        const media = header.format.toLowerCase()
        components.push({ type: 'header', parameters: [{ type: media, [media]: { id: '' } }] })
      }
    }
  }
  for (const part of template.components) {
    if (!isObject(part) || part.type !== 'BUTTONS' || !Array.isArray(part.buttons)) continue
    part.buttons.forEach((button, index) => {
      if (!isObject(button)) return
      const urlParameters = typeof button.url === 'string' ? [...button.url.matchAll(/{{\s*([^{}]+?)\s*}}/g)].map((match) => match[1].trim()) : []
      if (button.type === 'URL' && urlParameters.length && urlParameters.every((parameter) => parameter === '1')) components.push({ type: 'button', sub_type: 'url', index: String(index), parameters: [{ type: 'text', text: '' }] })
      if (button.type === 'COPY_CODE') components.push({ type: 'button', sub_type: 'copy_code', index: String(index), parameters: [{ type: 'coupon_code', coupon_code: '' }] })
    })
  }
  return components.length ? JSON.stringify(components, null, 2) : ''
}

function validateTemplateRequirements(template: CloudTemplate, components: JsonValue[]): void {
  const { fields } = templateInputs(template)
  const parts = components.filter(isObject)
  const named = template.parameter_format?.toUpperCase() === 'NAMED'
  for (const type of ['header', 'body'] as const) {
    const required = fields.filter((field) => field.component === type)
    if (!required.length) continue
    const matches = parts.filter((part) => part.type === type)
    const parameters = matches.length === 1 && Array.isArray(matches[0].parameters) ? matches[0].parameters.filter(isObject) : []
    if (named) {
      for (const field of required) {
        if (parameters.filter((parameter) => parameter.parameter_name === field.parameter).length !== 1) throw new Error(`Add the ${type} parameter_name "${field.parameter}" in Advanced JSON before sending.`)
      }
    } else if (parameters.length < required.length) throw new Error(`Add all ${required.length} required ${type} parameters in Advanced JSON before sending.`)
  }
  for (const part of template.components) {
    if (!isObject(part)) continue
    if (part.type === 'HEADER' && typeof part.format === 'string' && ['IMAGE', 'VIDEO', 'DOCUMENT', 'LOCATION'].includes(part.format)) {
      const format = part.format.toLowerCase()
      if (!parts.some((component) => component.type === 'header' && Array.isArray(component.parameters) && component.parameters.some((parameter) => isObject(parameter) && parameter.type === format))) throw new Error(`Add the required ${format} header parameter in Advanced JSON before sending.`)
    }
    if (part.type !== 'BUTTONS' || !Array.isArray(part.buttons)) continue
    part.buttons.forEach((button, index) => {
      if (!isObject(button)) return
      const subtype = button.type === 'URL' && typeof button.url === 'string' && button.url.includes('{{') ? 'url' : button.type === 'COPY_CODE' ? 'copy_code' : null
      if (subtype && !parts.some((component) => component.type === 'button' && component.sub_type === subtype && String(component.index) === String(index))) throw new Error(`Add the required ${subtype} button at index ${index} in Advanced JSON before sending.`)
    })
  }
}

/** Share the local checks between the send control and the explicit send action. */
export function templateSendComponents(template: CloudTemplate, values: Record<string, string>, raw: string): JsonValue[] | undefined {
  if (!raw.trim()) return templateInputComponents(template, values)
  const components = templateComponents(raw)
  const inputs = templateInputs(template)
  if ((inputs.advancedRequired || inputs.fields.length > 0) && !components?.length) throw new Error('Add the required component objects in Advanced JSON before sending.')
  if (components) validateTemplateRequirements(template, components)
  return components
}

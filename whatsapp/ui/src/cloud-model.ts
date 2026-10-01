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

export function templateComponents(raw: string): JsonValue[] | undefined {
  if (!raw.trim()) return undefined
  if (new TextEncoder().encode(raw).length > 65536) throw new Error('Template parameters exceed the 64 KiB limit.')
  let value: unknown
  try { value = JSON.parse(raw) } catch { throw new Error('Template parameters must be valid JSON.') }
  if (!Array.isArray(value) || !value.every(isObject)) throw new Error('Template parameters must be a JSON array of component objects.')
  return value
}

export function templateText(template: CloudTemplate): string {
  return template.components.flatMap((part) => isObject(part) && typeof part.text === 'string' ? [part.text] : []).join('\n\n')
}

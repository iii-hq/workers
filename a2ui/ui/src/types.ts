export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue }

export interface A2uiComponent {
  id: string
  component: string
  [key: string]: unknown
}

export interface ActionRecord {
  action_id: string
  name: string
  source_component_id: string
  context: JsonValue
  data_model?: JsonValue
  timestamp_ms: number
}

export interface SurfaceRecord {
  session_id: string
  surface_id: string
  protocol_version: string
  catalog_id: string
  title: string
  theme: JsonValue | null
  send_data_model: boolean
  components: A2uiComponent[]
  data_model: JsonValue
  revision: number
  created_at_ms: number
  updated_at_ms: number
  last_action: ActionRecord | null
  pinned: boolean
  bindings: LiveBinding[]
  history: SurfaceRevision[]
}

export interface SurfaceRevision { revision: number; title: string; updated_at_ms: number; reason: string }
export interface LiveBinding { id: string; trigger_type: string; config: JsonValue; target_path: string; event_path?: string }
export interface SurfaceTemplate { template_id: string; title: string; description: string; updated_at_ms: number }

export interface SurfaceSummary {
  surface_id: string
  title: string
  protocol_version: string
  catalog_id: string
  component_count: number
  revision: number
  updated_at_ms: number
  pinned: boolean
  binding_count: number
}

export interface CodeExport { format: string; surface_id: string; files: Array<{ path: string; content: string }> }

export interface SurfaceReceipt {
  session_id: string
  surface_id: string
  title: string
  status: 'active' | 'deleted'
  protocol_version: string
  catalog_id: string
  revision: number
  component_count: number
  page: string
}

export interface ActionResponse {
  accepted: boolean
  forwarded: boolean
  session_id: string
  surface_id: string
  revision: number
  turn_id?: string
  forward_error?: string
}

export interface ListResponse {
  session_id: string
  surfaces: SurfaceSummary[]
  count: number
}

export interface SurfaceExport {
  format: 'a2ui.surface'
  format_version: number
  protocol_version: string
  catalog_id: string
  surface_id: string
  title: string
  messages: JsonValue[]
}

export function unwrapEnvelope(value: unknown): unknown {
  if (value == null || typeof value !== 'object') return value
  const record = value as Record<string, unknown>
  if (record.details != null) return record.details
  if (Array.isArray(record.content) && record.content.length === 1) {
    const block = record.content[0]
    if (block != null && typeof block === 'object') {
      const text = (block as Record<string, unknown>).text
      if (typeof text === 'string') {
        try {
          return JSON.parse(text)
        } catch {
          return value
        }
      }
    }
  }
  return value
}

export function parseReceipt(value: unknown): SurfaceReceipt | null {
  const unwrapped = unwrapEnvelope(value)
  if (unwrapped == null || typeof unwrapped !== 'object') return null
  const record = unwrapped as Record<string, unknown>
  if (
    typeof record.session_id !== 'string' ||
    typeof record.surface_id !== 'string' ||
    typeof record.title !== 'string' ||
    typeof record.protocol_version !== 'string' ||
    typeof record.revision !== 'number' ||
    typeof record.component_count !== 'number'
  ) {
    return null
  }
  return record as unknown as SurfaceReceipt
}

import type { Host } from '@iii-dev/console-ui'
import type {
  JsonValue,
  ActionResponse,
  ListResponse,
  SurfaceExport,
  CodeExport,
  SurfaceRecord,
  SurfaceReceipt,
  SurfaceTemplate,
} from './types'
import { unwrapEnvelope } from './types'

async function call<T>(
  host: Host,
  functionId: string,
  payload: Record<string, unknown>,
): Promise<T> {
  return unwrapEnvelope(await host.iii.trigger(functionId, payload)) as T
}

export function mutateSurface(host: Host, functionId: string, sessionId: string, payload: Record<string, unknown>) {
  return call(host, functionId, { ...payload, session_id: sessionId })
}

export function exportCode(host: Host, sessionId: string, surfaceId: string, target: 'react' | 'worker'): Promise<CodeExport> {
  return call(host, 'a2ui::surface::export-code', { session_id: sessionId, surface_id: surfaceId, target })
}

export function listTemplates(host: Host, sessionId: string): Promise<{ templates: SurfaceTemplate[]; count: number }> {
  return call(host, 'a2ui::template::list', { session_id: sessionId })
}

export function applyBinding(host: Host, sessionId: string, surfaceId: string, bindingId: string, value: JsonValue): Promise<SurfaceReceipt> {
  return call(host, 'a2ui::binding::apply', { session_id: sessionId, surface_id: surfaceId, binding_id: bindingId, value })
}

export function listSurfaces(
  host: Host,
  sessionId: string,
): Promise<ListResponse> {
  return call(host, 'a2ui::surface::list', { session_id: sessionId })
}

export function getSurface(
  host: Host,
  sessionId: string,
  surfaceId: string,
): Promise<SurfaceRecord> {
  return call(host, 'a2ui::surface::get', {
    session_id: sessionId,
    surface_id: surfaceId,
  })
}

export function exportSurface(
  host: Host,
  sessionId: string,
  surfaceId: string,
): Promise<SurfaceExport> {
  return call(host, 'a2ui::surface::export', {
    session_id: sessionId,
    surface_id: surfaceId,
  })
}

export async function sendAction(
  host: Host,
  input: {
    sessionId: string
    surfaceId: string
    sourceComponentId: string
    actionId: string
    name: string
    context: JsonValue
    dataModel?: JsonValue
    expectedRevision: number
  },
): Promise<ActionResponse> {
  return call<ActionResponse>(host, 'a2ui::action', {
    session_id: input.sessionId,
    surface_id: input.surfaceId,
    source_component_id: input.sourceComponentId,
    action_id: input.actionId,
    name: input.name,
    context: input.context,
    data_model: input.dataModel,
    expected_revision: input.expectedRevision,
  })
}

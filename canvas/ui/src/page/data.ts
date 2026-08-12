/**
 * Thin typed wrappers over the worker's own `canvas::*` functions — the page
 * acts by invoking them through the tab's bus client (`host.iii.trigger`).
 * Results may arrive inside the harness content/details envelope when a
 * session relays them; `unwrapEnvelope` normalizes both shapes.
 */

import type { Host } from '@iii-dev/console-ui'
import {
  type CanvasDeleteResponse,
  type CanvasFormat,
  type CanvasListResponse,
  type CanvasRecord,
  unwrapEnvelope,
} from '../lib/types'

async function call<T>(
  host: Host,
  functionId: string,
  payload: Record<string, unknown>,
): Promise<T> {
  return unwrapEnvelope(await host.iii.trigger(functionId, payload)) as T
}

export function listCanvases(host: Host): Promise<CanvasListResponse> {
  return call(host, 'canvas::list', {})
}

export function getCanvas(host: Host, id: string): Promise<CanvasRecord> {
  return call(host, 'canvas::get', { id })
}

export function createCanvas(
  host: Host,
  input: { name: string; format: CanvasFormat; source: string },
): Promise<CanvasRecord> {
  return call(host, 'canvas::create', input)
}

export function updateCanvas(
  host: Host,
  id: string,
  source: string,
): Promise<CanvasRecord> {
  return call(host, 'canvas::update', { id, source })
}

export function deleteCanvas(
  host: Host,
  id: string,
): Promise<CanvasDeleteResponse> {
  return call(host, 'canvas::delete', { id })
}

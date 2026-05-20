import { mockBackend } from './mock'
import { realBackend } from './real'
import type { ChatBackend } from './types'

/**
 * Build-time flag: when truthy, ship the mock as the default backend (and
 * the playground page). When falsy, only the real backend is reachable, so
 * Rolldown tree-shakes `mockBackend` and every scenario module out of the
 * production chunk.
 */
const PLAYGROUND_ENABLED = !!import.meta.env.VITE_PLAYGROUND

export function getDefaultBackend(): ChatBackend {
  return PLAYGROUND_ENABLED ? mockBackend : realBackend
}

export type { ChatBackend, ChatStreamOptions, StreamEvent } from './types'

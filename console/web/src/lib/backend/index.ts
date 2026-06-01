import { mockBackend } from './mock'
import { realBackend } from './real'
import type { ChatBackend } from './types'

/**
 * Pick the chat backend by build mode: the mock streams canned bodies in dev
 * (`pnpm dev`), and the real backend stub is used for production builds. Vite
 * inlines `import.meta.env.DEV` as a literal, so Rolldown tree-shakes
 * `mockBackend` out of the production chunk.
 */
export function getDefaultBackend(): ChatBackend {
  return import.meta.env.DEV ? mockBackend : realBackend
}

export type { ChatBackend, ChatStreamOptions, StreamEvent } from './types'

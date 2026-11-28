// The `@iii-dev/console-ui/hooks` stand-in: `ui.narrow` drives the pane width; pane state lives in memory,
// and `ui.panes` holds what a pane had stored before it mounted (a reload).
import { useState } from 'react'
import { vi } from 'vitest'

export const ui = { narrow: false, panes: {} as Record<string, unknown> }
export const useContainerNarrow = () => ({ ref: () => undefined, narrow: ui.narrow })
/** A test sets `copyFlash.state = 'failed'` to see the failure. */
export const copyFlash = { state: 'idle' as 'idle' | 'copied' | 'failed' }
export const useCopyFlash = () => ({ state: copyFlash.state, copy: vi.fn() })
export const usePaneState = <T>(key: string, initial: T) =>
  useState<T>(key in ui.panes ? (ui.panes[key] as T) : initial)

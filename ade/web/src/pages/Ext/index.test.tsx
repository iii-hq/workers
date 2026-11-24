// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { registerExtPage } from '@/lib/ui-slots'
import {
  extPageIdForScreen,
  parseWorkspaceTabs,
  type TabScreen,
} from '@/lib/workspace-tabs'
import { ExtPage } from '.'

describe('ExtPage missing-worker state', () => {
  const cleanups: Array<() => void> = []

  afterEach(() => {
    for (const cleanup of cleanups.splice(0)) cleanup()
  })

  it.each(['tower', 'ext:tower'])(
    'restores a stored %s tab, waits without an error, and renders it when its worker registers',
    async (legacyScreen) => {
      const [restored] = parseWorkspaceTabs({
        tabs: [{ id: 'restored', screens: [legacyScreen as TabScreen] }],
      })
      const pageId = extPageIdForScreen(restored.screens[0] as TabScreen)
      expect(pageId).toBe('control-plane')
      const container = document.createElement('div')
      const root = createRoot(container)
      cleanups.push(() => root.unmount())

      await act(() =>
        root.render(<ExtPage pageId={pageId ?? ''} onRequestClose={vi.fn()} />),
      )

      expect(container.textContent).toContain('Waiting for worker page')
      expect(container.textContent).not.toContain('Extension page not loaded')
      expect(container.innerHTML).toContain('aria-label="close panel"')

      await act(() => {
        const unregister = registerExtPage({
          id: 'control-plane',
          title: 'Control plane',
          scope: 'control-plane',
          path: 'control-plane/page.js',
          render: () => <div>Registered control plane</div>,
        })
        cleanups.unshift(unregister)
      })

      expect(container.textContent).toContain('Registered control plane')
      expect(container.textContent).not.toContain('Waiting for worker page')
    },
  )
})

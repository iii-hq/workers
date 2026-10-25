// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import {
  Tooltip as ShadcnTooltip,
  TooltipContent as ShadcnTooltipContent,
  TooltipProvider as ShadcnTooltipProvider,
  TooltipTrigger as ShadcnTooltipTrigger,
} from '@/components/shadcn/tooltip'
import { INVERSE_INK, INVERSE_SURFACE_VARS } from '@/lib/neutral-surface'
import { Tooltip, TooltipProvider } from './Tooltip'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

beforeAll(() => {
  // The positioners measure their anchor; jsdom has no ResizeObserver.
  if (!('ResizeObserver' in globalThis)) {
    class ResizeObserverStub {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    ;(globalThis as { ResizeObserver?: unknown }).ResizeObserver =
      ResizeObserverStub
  }
})

afterEach(() => {
  document.body.innerHTML = ''
})

/**
 * A tooltip is the one inverse surface: ink behind, page colour on top. In
 * the dark theme that pill is light, so content that paints itself with
 * the page tokens (`text-ink`, a worker's `color: var(--color-ink)`) came
 * out white on white. Both primitives wrap their content in a
 * `display: contents` body that remaps the tokens the other way.
 */
describe('tooltip content inherits inverted ink tokens', () => {
  it('the console Tooltip wraps its label in the inverse body', async () => {
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    await act(async () =>
      root.render(
        <TooltipProvider>
          <Tooltip
            open
            label={<span className="text-ink judge-copy">help</span>}
          >
            <button type="button">i</button>
          </Tooltip>
        </TooltipProvider>,
      ),
    )
    const body = document.querySelector('[data-slot="tooltip-body"]')
    expect(body).not.toBeNull()
    expect(body?.className).toContain('contents')
    for (const token of INVERSE_INK.split(' ')) {
      expect(body?.className).toContain(token)
    }
    expect(body?.querySelector('.judge-copy')?.textContent).toBe('help')
    // The pill itself keeps its own colours and captures them for the body.
    const pill = body?.parentElement
    expect(pill?.className).toContain('bg-foreground')
    expect(pill?.className).toContain('text-background')
    for (const token of INVERSE_SURFACE_VARS.split(' ')) {
      expect(pill?.className).toContain(token)
    }
    // The pill must not remap its own ink — that would paint it page-colour.
    expect(pill?.className).not.toContain('[--color-ink:var(--inverse-fg)]')
    act(() => root.unmount())
  })

  it('the shadcn tooltip does the same', async () => {
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    await act(async () =>
      root.render(
        <ShadcnTooltipProvider>
          <ShadcnTooltip open>
            <ShadcnTooltipTrigger render={<button type="button">i</button>} />
            <ShadcnTooltipContent>
              <span className="text-ink-faint">help</span>
            </ShadcnTooltipContent>
          </ShadcnTooltip>
        </ShadcnTooltipProvider>,
      ),
    )
    const body = document.querySelector('[data-slot="tooltip-body"]')
    expect(body).not.toBeNull()
    expect(body?.className).toContain('contents')
    expect(body?.className).toContain('[--color-ink:var(--inverse-fg)]')
    expect(body?.className).toContain('[--foreground:var(--inverse-fg)]')
    const pill = body?.parentElement
    expect(pill?.getAttribute('data-slot')).toBe('tooltip-content')
    expect(pill?.className).toContain('bg-foreground')
    expect(pill?.className).toContain('[--inverse-bg:var(--foreground)]')
    act(() => root.unmount())
  })

  it('remaps the page tokens a worker stylesheet reaches for, not the pill colours', () => {
    for (const token of [
      '--color-ink',
      '--color-ink-faint',
      '--color-ink-ghost',
      '--color-edge',
      '--color-surface',
      '--color-panel-raised',
      '--foreground',
      '--muted-foreground',
    ]) {
      expect(INVERSE_INK).toContain(`[${token}:`)
    }
    // `background` is what the pill's own text and a Hint's key cap read.
    expect(INVERSE_INK).not.toContain('[--background:')
    expect(INVERSE_INK).not.toContain('[--color-bg:')
  })
})

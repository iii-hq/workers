// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import { SessionObservers } from './SessionObservers'

function observer(
  overrides: Partial<SessionObserverInfo>,
): SessionObserverInfo {
  return {
    id: 'obs_1',
    functionId: 'state::on-changed',
    delivery: { kind: 'notify' },
    filters: {
      'input.scope': { $eq: 'review' },
      'input.key': { $eq: 'agent-1/findings' },
    },
    once: true,
    ...overrides,
  }
}

const rows: SessionObserverInfo[] = [
  observer({ id: 'sub_live', label: 'Live watch' }),
  observer({ id: 'sub_spent_1', label: 'Spent one', fired: true }),
  observer({ id: 'sub_spent_2', label: 'Spent two', fired: true }),
]

let root: Root | null = null
let host: HTMLDivElement

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  localStorage.clear()
  host = document.body.appendChild(document.createElement('div'))
  root = createRoot(host)
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = null
  host.remove()
})

const render = (onDispose = vi.fn()) =>
  act(async () =>
    root?.render(
      <TooltipProvider>
        <SessionObservers
          observers={rows}
          onDispose={onDispose}
          defaultExpanded
        />
      </TooltipProvider>,
    ),
  )

const text = () => host.textContent ?? ''

describe('SessionObservers clearing inactive rows', () => {
  it('clears every inactive row, keeps the live one, and remembers it across a reload', async () => {
    const onDispose = vi.fn()
    await render(onDispose)
    expect(text()).toContain('2 inactive')

    await act(async () =>
      host
        .querySelector<HTMLButtonElement>(
          '[aria-label="clear 2 inactive observers"]',
        )
        ?.click(),
    )
    expect(text()).toContain('Live watch')
    expect(text()).not.toContain('Spent one')
    expect(text()).not.toContain('inactive')
    // Nothing live was torn down.
    expect(onDispose).not.toHaveBeenCalled()

    // A reload rebuilds the rows from the transcript; the dismissal holds.
    await act(async () => root?.unmount())
    root = createRoot(host)
    await render()
    expect(text()).toContain('Live watch')
    expect(text()).not.toContain('Spent two')
  })

  it('opens the inactive group on its own disclosure', async () => {
    await render()
    const toggle = [...host.querySelectorAll('button')].find(
      (button) => button.textContent === '2 inactive',
    )
    expect(toggle?.getAttribute('aria-expanded')).toBe('false')
    await act(async () => toggle?.click())
    expect(toggle?.getAttribute('aria-expanded')).toBe('true')
  })
})

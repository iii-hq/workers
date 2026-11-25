// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { limitCardModel } from '@/lib/limit-card'
import {
  accountsData,
  usageFixture,
} from '@/stories/fixtures/claude-accounts-fixtures'
import type { SystemMessage } from '@/types/chat'
import { LimitCard, type LimitCardProps } from './LimitCard'

/* Sat 2026-11-21 10:00 UTC. */
const NOW = Date.UTC(2026, 10, 21, 10, 0)
const HOUR = 3_600_000
const TIME = { now: NOW, timeZone: 'UTC', hour24: false }
const data = accountsData(usageFixture(NOW))

function failed(window: 'five_hour' | 'week' | null): SystemMessage {
  return {
    id: 'f1',
    role: 'system',
    kind: 'turn-failure',
    content: 'This Claude Code account reached its usage limit',
    createdAt: NOW,
    failure: {
      usageLimit: {
        window,
        resetsAtMs: NOW + 6 * HOUR + 40 * 60_000,
        account: 'personal',
      },
    },
  }
}

function model(failure: SystemMessage | null, resolved = 'personal') {
  const m = limitCardModel({
    sessionId: 's1',
    failure,
    idle: true,
    resolved,
    account: data.account,
  })
  if (!m) throw new Error('no card')
  return m
}

let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
})

afterEach(() => {
  act(() => root?.unmount())
  root = null
  document.body.innerHTML = ''
  vi.unstubAllGlobals()
})

function render(
  props: Partial<LimitCardProps> & Pick<LimitCardProps, 'model'>,
) {
  const handlers = {
    onContinue: vi.fn<(account: string) => Promise<void>>(() =>
      Promise.resolve(),
    ),
    onMakeGlobal: vi.fn<(account: string) => Promise<void>>(() =>
      Promise.resolve(),
    ),
    onOpenAccounts: vi.fn(),
    onDismiss: vi.fn(),
  }
  root = createRoot(document.body.appendChild(document.createElement('div')))
  act(() =>
    root?.render(
      <TooltipProvider delayDuration={0}>
        <LimitCard data={data} time={TIME} {...handlers} {...props} />
      </TooltipProvider>,
    ),
  )
  return handlers
}

const card = () => document.querySelector('[data-limit-card]') as HTMLElement
const q = (selector: string) =>
  document.querySelector(selector) as HTMLButtonElement | null

describe('the limit card', () => {
  it('says which window is full, when it resets, and continues in the account with the most room', async () => {
    const h = render({ model: model(failed('five_hour')) })
    expect(card().dataset.limitCard).toBe('failed')
    expect(card().querySelector('h3')?.textContent).toBe(
      'Personal reached its 5-hour limit',
    )
    expect(card().textContent).toContain('Resets 4:40 PM')
    expect(card().textContent).toContain(
      'Changing account starts a new prompt cache for this conversation.',
    )
    const go = q('[data-limit-continue]')
    expect(go?.textContent).toBe('Continue in Work')
    await act(async () => go?.click())
    expect(h.onContinue).toHaveBeenCalledWith('default')
    // Only a conversation that follows the global offers to move the global.
    expect(q('[data-limit-make-global]')).toBeNull()
  })

  it('offers to make the best account global when the conversation follows the global', async () => {
    const h = render({ model: model(failed('week')), followsGlobal: true })
    expect(card().querySelector('h3')?.textContent).toBe(
      'Personal reached its weekly limit',
    )
    const make = q('[data-limit-make-global]')
    expect(make?.textContent).toBe('Make Work the global')
    await act(async () => make?.click())
    expect(h.onMakeGlobal).toHaveBeenCalledWith('default')
  })

  it('lists the other accounts with their use, signed-out ones disabled', () => {
    render({ model: model(failed('five_hour')) })
    const trigger = q('[data-limit-other]') as HTMLButtonElement
    act(() => {
      trigger.dispatchEvent(
        new PointerEvent('pointerdown', { bubbles: true, button: 0 }),
      )
      trigger.click()
    })
    const rows = [...document.querySelectorAll('[data-limit-account]')]
    expect(rows.map((row) => row.getAttribute('data-limit-account'))).toEqual([
      'default',
      'side-project',
    ])
    expect(rows[0].textContent).toContain('5 hours 38% · This week 64%')
    expect(rows[1].textContent).toContain('Not signed in')
    expect(rows[1].hasAttribute('data-disabled')).toBe(true)
    expect(document.body.textContent).toContain('Claude Code accounts…')
  })

  it('shows on a reading at 100 % while idle', () => {
    render({ model: model(null) })
    expect(card().dataset.limitCard).toBe('reading')
    expect(card().querySelector('h3')?.textContent).toBe(
      'Personal reached its 5-hour limit',
    )
    expect(card().textContent).toContain('Resets 11:00 AM')
  })

  it("keeps a side chat on its account: no choices, the side chat's hint", () => {
    render({ model: model(failed('five_hour')), sideChat: true })
    expect(q('[data-limit-continue]')).toBeNull()
    expect(q('[data-limit-other]')).toBeNull()
    expect(card().textContent).toContain(
      "This side chat stays on Personal so it keeps the conversation's cache.",
    )
  })

  it('names a removed account and offers the others', () => {
    const removed: SystemMessage = {
      ...failed(null),
      failure: { unknownAccount: 'old-team' },
    }
    render({ model: model(removed), followsGlobal: true })
    expect(card().dataset.limitCard).toBe('unknown')
    expect(card().querySelector('h3')?.textContent).toBe(
      'old-team is no longer a Claude Code account',
    )
    expect(card().textContent).toContain('Choose another account to continue.')
    expect(q('[data-limit-continue]')?.textContent).toBe('Continue in Work')
    expect(q('[data-limit-make-global]')).toBeNull()
  })

  it("shows the worker's refusal and puts the card away with the X", async () => {
    const h = render({ model: model(failed('five_hour')) })
    h.onContinue.mockRejectedValueOnce(
      new Error('{"code":"not_signed_in","message":"Work is signed out"}'),
    )
    await act(async () => q('[data-limit-continue]')?.click())
    expect(card().querySelector('[role="alert"]')?.textContent).toBe(
      'Work is signed out',
    )
    act(() => q('[data-limit-dismiss]')?.click())
    expect(h.onDismiss).toHaveBeenCalled()
  })
})

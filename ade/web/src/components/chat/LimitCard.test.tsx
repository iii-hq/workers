// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import {
  type ClaudeAccounts,
  type GlobalResult,
  parseUsageSnapshot,
} from '@/lib/backend/claude-accounts'
import { limitCardModel } from '@/lib/limit-card'
import {
  accountsData,
  rawUsage,
  usageFixture,
} from '@/stories/fixtures/claude-accounts-fixtures'
import type { SystemMessage } from '@/types/chat'
import { LimitCard, type LimitCardProps } from './LimitCard'

/* Sat 2026-11-21 10:00 UTC. */
const NOW = Date.UTC(2026, 10, 21, 10, 0)
const HOUR = 3_600_000
const TIME = { now: NOW, timeZone: 'UTC', hour24: false }
const data = accountsData(usageFixture(NOW))

/** The fixture's accounts with tmux's snapshot changed (`claude.*`). */
function dataWith(claude: Record<string, unknown>): ClaudeAccounts {
  const raw = rawUsage(NOW, claude)
  return accountsData(parseUsageSnapshot(raw))
}

/** Personal the global one (the conversations that follow it ran out). */
const onPersonal = dataWith({ global: 'personal' })

/** Work's week full too: no account has room. */
function noRoom(): ClaudeAccounts {
  const raw = rawUsage(NOW)
  const work = raw.claude.accounts[0] as { usage: { week: unknown } }
  work.usage.week = { used_percent: 100, resets_at_ms: NOW + 50 * HOUR }
  return accountsData(parseUsageSnapshot(raw))
}

function failed(
  window: 'five_hour' | 'week' | null,
  resetsAtMs = NOW + 6 * HOUR + 40 * 60_000,
): SystemMessage {
  return {
    id: 'f1',
    role: 'system',
    kind: 'turn-failure',
    content: 'This Claude Code account reached its usage limit',
    createdAt: NOW,
    failure: { usageLimit: { window, resetsAtMs, account: 'personal' } },
  }
}

function model(
  failure: SystemMessage | null,
  { resolved = 'personal', now = NOW, of = data } = {},
) {
  const m = limitCardModel({
    sessionId: 's1',
    failure,
    idle: true,
    resolved,
    account: of.account,
    now,
  })
  if (!m) throw new Error('no card')
  return m
}

const switched: GlobalResult = {
  global: 'default',
  state: 'switched',
  following: 1,
  restarted: [],
  afterTurn: [],
  waitingFor: [],
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
    onPin: vi.fn<(account: string) => Promise<void>>(() => Promise.resolve()),
    onSetGlobal: vi.fn<(account: string) => Promise<GlobalResult>>(() =>
      Promise.resolve(switched),
    ),
    onRetry: vi.fn<() => Promise<void>>(() => Promise.resolve()),
    onOpenAccounts: vi.fn(),
    onDismiss: vi.fn(),
  }
  root = createRoot(document.body.appendChild(document.createElement('div')))
  const draw = (next: Partial<LimitCardProps>) =>
    act(() =>
      root?.render(
        <TooltipProvider delayDuration={0}>
          <LimitCard
            data={data}
            time={TIME}
            {...handlers}
            {...props}
            {...next}
          />
        </TooltipProvider>,
      ),
    )
  draw({})
  return { ...handlers, rerender: draw }
}

const card = () => document.querySelector('[data-limit-card]') as HTMLElement
const q = (selector: string) =>
  document.querySelector(selector) as HTMLButtonElement | null
const openOther = () => {
  const trigger = q('[data-limit-other]') as HTMLButtonElement
  act(() => {
    trigger.dispatchEvent(
      new PointerEvent('pointerdown', { bubbles: true, button: 0 }),
    )
    trigger.click()
  })
}

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
    // Pins the conversation, then re-runs the failed turn.
    expect(h.onPin).toHaveBeenCalledWith('default')
    expect(h.onRetry).toHaveBeenCalledTimes(1)
    // Only a conversation that follows the global offers to move the global.
    expect(q('[data-limit-make-global]')).toBeNull()
  })

  it('keeps its buttons off after a re-run until the card changes', async () => {
    const h = render({ model: model(failed('five_hour')) })
    await act(async () => q('[data-limit-continue]')?.click())
    expect(h.onRetry).toHaveBeenCalledTimes(1)
    // The turn hasn't started yet: a second press would answer "busy".
    expect(q('[data-limit-continue]')?.disabled).toBe(true)
    expect(q('[data-limit-other]')?.disabled).toBe(true)
    await act(async () => q('[data-limit-continue]')?.click())
    expect(h.onRetry).toHaveBeenCalledTimes(1)
    // The re-run failed again: another failure, another card.
    h.rerender({ model: model({ ...failed('five_hour'), id: 'f2' }) })
    expect(q('[data-limit-continue]')?.disabled).toBe(false)
  })

  it('offers no "Continue in" when no account has room, and lists them all in Other…', () => {
    const full = noRoom()
    render({ model: model(failed('week'), { of: full }), data: full })
    expect(q('[data-limit-continue]')).toBeNull()
    expect(q('[data-limit-make-global]')).toBeNull()
    openOther()
    const work = document.querySelector('[data-limit-account="default"]')
    expect(work?.textContent).toContain('5 hours 38% · This week 100%')
    expect(work?.hasAttribute('data-disabled')).toBe(false)
  })

  it('makes the best account global, then re-runs the turn', async () => {
    const h = render({
      model: model(failed('week')),
      data: onPersonal,
      followsGlobal: true,
    })
    expect(card().querySelector('h3')?.textContent).toBe(
      'Personal reached its weekly limit',
    )
    const make = q('[data-limit-make-global]')
    expect(make?.textContent).toBe('Make Work the global')
    await act(async () => make?.click())
    expect(h.onSetGlobal).toHaveBeenCalledWith('default')
    expect(h.onPin).not.toHaveBeenCalled()
    expect(h.onRetry).toHaveBeenCalledTimes(1)
  })

  it('waits with tmux when the switch waits: its words, no re-run, Continue in still there', async () => {
    const h = render({
      model: model(failed('week')),
      data: onPersonal,
      followsGlobal: true,
    })
    h.onSetGlobal.mockResolvedValueOnce({
      ...switched,
      global: 'personal',
      state: 'waiting',
      waitingFor: ['%3'],
    })
    await act(async () => q('[data-limit-make-global]')?.click())
    expect(h.onRetry).not.toHaveBeenCalled()
    expect(card().querySelector('[data-limit-waiting]')?.textContent).toBe(
      'Switching to Work when 1 window pinned to it finishes its turn.',
    )
    expect(q('[data-limit-continue]')?.disabled).toBe(false)
    // tmux's pushed snapshot says the same while it waits…
    const waiting = dataWith({
      global: 'personal',
      pending_global: {
        account: 'default',
        by: 'person',
        waiting_for: ['%3', '%5'],
        at_ms: NOW,
      },
    })
    h.rerender({ data: waiting })
    expect(card().querySelector('[data-limit-waiting]')?.textContent).toBe(
      'Switching to Work when 2 windows pinned to it finish their turn.',
    )
    expect(q('[data-limit-make-global]')?.disabled).toBe(true)
    expect(h.onRetry).not.toHaveBeenCalled()
    // …and once the switch is done, the failed turn runs again.
    h.rerender({ data: dataWith({ global: 'default' }) })
    expect(h.onRetry).toHaveBeenCalledTimes(1)
    expect(card().querySelector('[data-limit-waiting]')).toBeNull()
  })

  it('drops a waiting switch tmux called off, without a re-run', async () => {
    const h = render({
      model: model(failed('week')),
      data: onPersonal,
      followsGlobal: true,
    })
    h.onSetGlobal.mockResolvedValueOnce({
      ...switched,
      global: 'personal',
      state: 'waiting',
      waitingFor: [],
    })
    await act(async () => q('[data-limit-make-global]')?.click())
    h.rerender({
      data: dataWith({
        global: 'personal',
        pending_global: {
          account: 'default',
          by: 'person',
          waiting_for: [],
          at_ms: NOW,
        },
      }),
    })
    h.rerender({ data: dataWith({ global: 'personal' }) })
    expect(card().querySelector('[data-limit-waiting]')).toBeNull()
    expect(h.onRetry).not.toHaveBeenCalled()
  })

  it("shows a refused switch's words and lets the buttons be pressed again", async () => {
    const h = render({
      model: model(failed('week')),
      data: onPersonal,
      followsGlobal: true,
    })
    h.onSetGlobal.mockRejectedValueOnce(
      new Error('Claude Code is renewing its sign-in; try again in a moment.'),
    )
    await act(async () => q('[data-limit-make-global]')?.click())
    expect(card().querySelector('[role="alert"]')?.textContent).toBe(
      'Claude Code is renewing its sign-in; try again in a moment.',
    )
    expect(h.onRetry).not.toHaveBeenCalled()
    expect(q('[data-limit-make-global]')?.disabled).toBe(false)
  })

  it('once the window reset: "The limit has reset" and "Try again" on the same account', async () => {
    const reset = model(failed('five_hour', NOW + HOUR), {
      now: NOW + HOUR + 1,
    })
    const h = render({ model: reset, followsGlobal: true })
    expect(card().textContent).toContain('The limit has reset')
    expect(card().textContent).not.toContain('Resets')
    expect(q('[data-limit-continue]')).toBeNull()
    expect(q('[data-limit-make-global]')).toBeNull()
    // The accounts are still in Other….
    openOther()
    expect(
      document.querySelector('[data-limit-account="default"]'),
    ).not.toBeNull()
    act(() =>
      document.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
      ),
    )
    const again = q('[data-limit-retry]')
    expect(again?.textContent).toBe('Try again')
    await act(async () => again?.click())
    expect(h.onRetry).toHaveBeenCalledTimes(1)
    expect(h.onPin).not.toHaveBeenCalled()
  })

  it('lists the other accounts with their use, signed-out ones disabled', () => {
    render({ model: model(failed('five_hour')) })
    openOther()
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

  it('shows on a reading at 100 % while idle; continuing pins without a re-run', async () => {
    const h = render({ model: model(null) })
    expect(card().dataset.limitCard).toBe('reading')
    expect(card().querySelector('h3')?.textContent).toBe(
      'Personal reached its 5-hour limit',
    )
    expect(card().textContent).toContain('Resets 11:00 AM')
    await act(async () => q('[data-limit-continue]')?.click())
    expect(h.onPin).toHaveBeenCalledWith('default')
    expect(h.onRetry).not.toHaveBeenCalled()
    expect(q('[data-limit-continue]')?.disabled).toBe(false)
  })

  it("keeps a side chat on its account: no choices, the side chat's hint", () => {
    render({ model: model(failed('five_hour')), locked: 'side-chat' })
    expect(q('[data-limit-continue]')).toBeNull()
    expect(q('[data-limit-other]')).toBeNull()
    expect(card().textContent).toContain(
      "This side chat stays on Personal so it keeps the conversation's cache.",
    )
  })

  it("keeps a sub-agent on its conversation's account, in its own words", () => {
    render({ model: model(failed('five_hour')), locked: 'sub-agent' })
    expect(q('[data-limit-continue]')).toBeNull()
    expect(q('[data-limit-other]')).toBeNull()
    expect(card().textContent).toContain(
      "This sub-agent stays on its conversation's account.",
    )
    expect(card().textContent).not.toContain('side chat')
  })

  it('sends a locked conversation whose account was removed back to the main one', () => {
    const removed = model({
      ...failed(null),
      failure: { unknownAccount: 'old-team' },
    })
    render({ model: removed, locked: 'side-chat' })
    expect(card().querySelector('h3')?.textContent).toBe(
      'old-team is no longer a Claude Code account',
    )
    const said = () => card().querySelectorAll('p')
    expect([...said()].map((p) => p.textContent)).toEqual([
      "Its account was removed from tmux, and a side chat keeps its conversation's account. Continue in the main conversation.",
    ])
    expect(q('[data-limit-continue]')).toBeNull()
    act(() => root?.unmount())
    render({ model: removed, locked: 'sub-agent' })
    expect([...said()].map((p) => p.textContent)).toEqual([
      'Its account was removed from tmux. Continue in the main conversation.',
    ])
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
    h.onPin.mockRejectedValueOnce(
      new Error('{"code":"not_signed_in","message":"Work is signed out"}'),
    )
    await act(async () => q('[data-limit-continue]')?.click())
    expect(card().querySelector('[role="alert"]')?.textContent).toBe(
      'Work is signed out',
    )
    expect(h.onRetry).not.toHaveBeenCalled()
    expect(q('[data-limit-continue]')?.disabled).toBe(false)
    act(() => q('[data-limit-dismiss]')?.click())
    expect(h.onDismiss).toHaveBeenCalled()
  })
})

// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { RadioGroup } from '@/components/shadcn/radio-group'
import { TooltipProvider } from '@/components/ui/Tooltip'
import {
  accountsData,
  splitsFixture,
  usageFixture,
} from '@/stories/fixtures/claude-accounts-fixtures'
import { AccountCard, type AccountCardProps } from './AccountCard'

/* Sat 2026-11-21 10:00 UTC. */
const NOW = Date.UTC(2026, 10, 21, 10, 0)
const TIME = { now: NOW, timeZone: 'UTC', hour24: true }
const data = accountsData(usageFixture(NOW), splitsFixture(NOW))

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

function render(id: string, props: Partial<AccountCardProps> = {}) {
  const account = data.account(id)
  if (!account) throw new Error(`no account ${id}`)
  root = createRoot(document.body.appendChild(document.createElement('div')))
  act(() =>
    root?.render(
      <TooltipProvider delayDuration={0}>
        <RadioGroup value="default">
          <AccountCard
            account={account}
            isGlobal={id === 'default'}
            time={TIME}
            split={data.split(id)}
            {...props}
          />
        </RadioGroup>
      </TooltipProvider>,
    ),
  )
  return document.querySelector(`[data-account-card="${id}"]`) as HTMLElement
}

const legend = (card: HTMLElement) =>
  [...card.querySelectorAll('[data-split-item]')].map((item) => [
    item.getAttribute('data-split-item'),
    item.textContent?.trim(),
  ])

describe('an account card', () => {
  it('shows the global account with its plan, meters, reset line and week by project', () => {
    const card = render('default')
    expect(card.dataset.global).toBe('true')
    expect(card.textContent).toContain('global')
    expect(card.textContent).toContain('Team')
    expect(card.textContent).toContain('5 hours')
    expect(card.textContent).toContain('This week')
    expect(card.textContent).toContain('Resets Mon 12:00 · Opus 41%')
    expect(card.textContent).toContain(
      'This week by project (from session tokens)',
    )
    expect(legend(card)).toEqual([
      ['blue', 'acme-web 33%'],
      ['purple', 'mobile 12%'],
      ['no-project', 'no project 5%'],
      ['outside', 'Outside the Chat 14%'],
    ])
    expect(card.querySelector('[data-split-note]')).toBeNull()
  })

  it('leaves out the grey segment when outside the Chat was not measured', () => {
    const card = render('personal')
    expect(card.dataset.global).toBeUndefined()
    expect(card.textContent).toContain('Max 20×')
    expect(legend(card)).toEqual([['green', 'billing-api 15%']])
    expect(card.querySelector('[data-split-note]')?.textContent).toBe(
      "Usage outside the Chat isn't measured here.",
    )
  })

  it('offers the sign-in of an account that never signed in, without meters or split', () => {
    const onSignIn = vi.fn()
    const card = render('side-project', { onSignIn })
    expect(card.textContent).toContain('Not signed in')
    expect(card.textContent).not.toContain('This week')
    expect(card.querySelector('[data-account-split]')).toBeNull()
    const button = [...card.querySelectorAll('button')].find(
      (b) => b.textContent === 'Sign in',
    )
    act(() => button?.click())
    expect(onSignIn).toHaveBeenCalled()
  })

  it('hides the split when the caller does not ask for it', () => {
    const card = render('default', { split: undefined })
    expect(card.querySelector('[data-account-split]')).toBeNull()
  })
})

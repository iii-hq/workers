// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { seedClaudeAccountsForTests } from '@/lib/backend/claude-accounts'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import {
  rawUsage,
  usageFixture,
} from '@/stories/fixtures/claude-accounts-fixtures'
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

// The store is seeded; nothing here reads an engine.
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: vi.fn(async () => ({})) }),
}))

import { UsageFooter } from './UsageFooter'

const NOW = Date.now()

let root: Root | null = null
let host: HTMLElement | null = null

function seed(overrides: Record<string, unknown> = {}) {
  seedClaudeAccountsForTests({ usage: usageFixture(NOW, overrides) })
}

/** The fixture's accounts with Work's and Personal's weeks replaced. */
function withWeeks(work: number, personal: number) {
  const accounts = rawUsage(NOW).claude.accounts.map((a) =>
    a.id === 'default' || a.id === 'personal'
      ? {
          ...a,
          usage: {
            ...a.usage,
            week: {
              used_percent: a.id === 'default' ? work : personal,
              resets_at_ms: NOW + 3_600_000,
            },
          },
        }
      : a,
  )
  return { accounts }
}

function render() {
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <TooltipProvider>
        <UsageFooter />
      </TooltipProvider>,
    )
  })
}

const button = () =>
  document.querySelector<HTMLButtonElement>('[data-usage-rings]')
const ring = (id: string) =>
  document.querySelector(`[data-footer-account="${id}"] svg`)

beforeEach(() => {
  setChatCapabilitiesForTests({
    harnessRetry: true,
    tmuxAccounts: true,
    tmuxCredential: true,
  })
})

afterEach(() => {
  act(() => root?.unmount())
  host?.remove()
  root = null
  host = null
  setChatCapabilitiesForTests(null)
  seedClaudeAccountsForTests({ usage: null })
})

it('is one button: the mark and a ring per account, no text', () => {
  seed()
  render()
  const b = button()
  expect(b).not.toBeNull()
  expect(b?.textContent).toBe('')
  expect(b?.getAttribute('aria-label')).toBe(
    'Claude Code usage this week: Work, global, 64%; Personal, 22%; ' +
      'Side project, Not signed in. Opens Claude Code accounts.',
  )
  expect(b?.querySelector('[data-provider-icon="mark"]')).not.toBeNull()
  const rings = [...(b?.querySelectorAll('[data-footer-account]') ?? [])]
  expect(rings.map((r) => r.getAttribute('data-footer-account'))).toEqual([
    'default',
    'personal',
    'side-project',
  ])
  for (const r of rings) expect(r.getAttribute('aria-hidden')).toBe('true')
  expect(
    document.querySelector('[data-usage-footer]')?.textContent,
  ).not.toMatch(/Usage this week|global|%/)
})

it('puts the global account first', () => {
  seed({ global: 'side-project' })
  render()
  expect(
    [...document.querySelectorAll('[data-footer-account]')].map((r) =>
      r.getAttribute('data-footer-account'),
    ),
  ).toEqual(['side-project', 'default', 'personal'])
  expect(button()?.getAttribute('aria-label')).toMatch(
    /^Claude Code usage this week: Side project, global, Not signed in; Work, 64%;/,
  )
})

it("draws each week's arc in the account's colour, the track by level", () => {
  seed(withWeeks(85, 100))
  render()
  const work = ring('default')
  expect(work?.getAttribute('data-usage-ring')).toBe('85')
  expect(work?.getAttribute('data-usage-level')).toBe('high')
  const [workTrack, workArc] = work?.querySelectorAll('circle') ?? []
  expect(workTrack?.getAttribute('class')).toContain('stroke-warn/40')
  expect(workArc?.getAttribute('class')).toContain('stroke-account-blue')
  expect(workArc?.getAttribute('stroke-linecap')).toBe('round')
  expect(workArc?.getAttribute('transform')).toBe('rotate(-90 12 12)')
  // 85 % of 2π·9.
  expect(workArc?.getAttribute('stroke-dasharray')).toBe('48.07 56.55')

  // At 100 % the arc closes and the track turns red.
  const personal = ring('personal')
  expect(personal?.getAttribute('data-usage-level')).toBe('full')
  const [track, arc] = personal?.querySelectorAll('circle') ?? []
  expect(track?.getAttribute('class')).toContain('stroke-alert/40')
  expect(arc?.getAttribute('class')).toContain('stroke-account-violet')
  expect(arc?.hasAttribute('stroke-dasharray')).toBe(false)

  // Not signed in: the track alone.
  const side = ring('side-project')
  expect(side?.getAttribute('data-usage-ring')).toBe('none')
  expect(side?.querySelectorAll('circle')).toHaveLength(1)
  expect(side?.querySelector('circle')?.getAttribute('class')).toContain(
    'stroke-border',
  )
})

it('opens Claude Code accounts', () => {
  seed()
  render()
  act(() => button()?.click())
  expect(
    document.querySelector('[role="dialog"]')?.getAttribute('aria-label') ??
      document.querySelector('[role="dialog"]')?.textContent,
  ).toMatch(/Claude Code accounts/)
})

it('shows nothing without accounts', () => {
  seed({ accounts: [] })
  render()
  expect(document.querySelector('[data-usage-footer]')).toBeNull()
})

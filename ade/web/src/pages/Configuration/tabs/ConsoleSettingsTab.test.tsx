// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { seedChatProjectsForTests } from '@/hooks/use-chat-projects'
import { seedChatSettingsForTests } from '@/hooks/use-chat-settings'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import { GENERAL_ANCHOR } from '../lib/general-settings-index'
import { ConsoleSettingsTab } from './ConsoleSettingsTab'
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

// No engine here: every call fails, so the seeded stores stand.
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({
    call: async () => {
      throw new Error('no engine in tests')
    },
    on: () => () => {},
    observe: () => () => {},
    addConnectionStateListener: () => () => {},
  }),
}))

let root: Root | null = null
let host: HTMLElement | null = null

beforeEach(() => {
  setChatCapabilitiesForTests({
    chatSettings: true,
    setDescription: true,
    autocompact: true,
  })
  seedChatSettingsForTests({
    descriptions: { agent_writes: true },
    autocompact: { mode: 'tokens', tokens: 400_000 },
  })
  seedChatProjectsForTests([])
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  const queries = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  act(() => {
    root?.render(
      <QueryClientProvider client={queries}>
        <TooltipProvider delayDuration={0}>
          <ConsoleSettingsTab theme="system" onThemeChange={vi.fn()} />
        </TooltipProvider>
      </QueryClientProvider>,
    )
  })
})

afterEach(() => {
  act(() => root?.unmount())
  host?.remove()
  root = null
  setChatCapabilitiesForTests(null)
  seedChatSettingsForTests(null)
  seedChatProjectsForTests(null)
})

describe('Settings › Chat on a phone (390 px)', () => {
  // The env-p3 acceptance: "Session descriptions" kept its switch label on
  // one line beside a text column squeezed to a letter a line. The row
  // stacks its control under a 30rem container, as the Auto-compact row
  // beside it does.
  it('stacks the Session descriptions control under 30rem', () => {
    const row = document.getElementById(GENERAL_ANCHOR.sessionDescriptions)
    expect(row?.textContent).toContain('Session descriptions')
    expect(row?.textContent).toContain('Agents keep descriptions up to date')
    expect(row?.getAttribute('data-layout')).toBe('auto')
    expect(
      document
        .getElementById(GENERAL_ANCHOR.autocompact)
        ?.getAttribute('data-layout'),
    ).toBe('auto')
  })
})

// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { seedChatProjectsForTests } from '@/hooks/use-chat-projects'
import { seedChatSettingsForTests } from '@/hooks/use-chat-settings'
import type {
  AutocompactLogEntry,
  AutocompactModel,
} from '@/lib/backend/autocompact'
import type { ChatProject } from '@/lib/backend/projects'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import { setSessionListHasMoreForTests } from '@/lib/sessions/api'
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({
  /** `harness::chat-settings::set` and anything else the dialog calls. */
  call: vi.fn(),
  conversations: [] as unknown[],
  openConversation: vi.fn(),
  saveDetails: vi.fn(),
}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: mocks.call }),
}))
vi.mock('@/lib/conversations-context', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useConversationsCtxOptional: () => ({
    conversations: mocks.conversations,
    openConversation: mocks.openConversation,
    saveDetails: mocks.saveDetails,
  }),
}))

import { AutocompactDialog, type AutocompactSource } from './AutocompactDialog'

const PROJECTS: ChatProject[] = [
  {
    id: 'prj_web',
    name: 'acme-web',
    color: 'blue',
    folders: ['/w'],
    autocompact: { mode: 'tokens', tokens: 160_000 },
    created_at_ms: 0,
    updated_at_ms: 0,
  },
  {
    id: 'prj_billing',
    name: 'billing-api',
    color: 'green',
    folders: ['/b'],
    created_at_ms: 0,
    updated_at_ms: 0,
  },
  {
    id: 'prj_mobile',
    name: 'mobile',
    color: 'rose',
    folders: ['/m'],
    autocompact: { mode: 'off' },
    created_at_ms: 0,
    updated_at_ms: 0,
  },
]

const MODELS: AutocompactModel[] = [
  {
    provider: 'anthropic',
    model: 'claude-opus-5-5',
    name: 'Opus 5.5',
    context_window: 200_000,
    usable: 116_000,
    last_used_ms: 1,
  },
  {
    provider: 'anthropic',
    model: 'claude-sonnet-5-5',
    name: 'Sonnet 5.5',
    context_window: 1_000_000,
    usable: 916_000,
    last_used_ms: 2,
  },
]

const LOG: AutocompactLogEntry[] = [
  {
    session_id: 's-ledger',
    title: 'Ledger reconciliation',
    at_ms: Date.now() - 5 * 60_000,
    trigger: 'auto',
    threshold_tokens: 600_000,
    context_before: 601_000,
    context_after: 40_000,
    rule: { scope: 'session' },
  },
]

const GLOBAL = {
  mode: 'tokens' as const,
  tokens: 400_000,
  instructions: 'Keep the PRs.',
}

let root: Root | null = null
let host: HTMLElement | null = null

function source(over: Partial<AutocompactSource> = {}): AutocompactSource {
  return {
    models: async () => MODELS,
    log: async () => LOG,
    ...over,
  }
}

async function render(over: Partial<AutocompactSource> = {}) {
  const onOpenChange = vi.fn()
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  await act(async () => {
    root?.render(
      <TooltipProvider delayDuration={0}>
        <AutocompactDialog
          open
          onOpenChange={onOpenChange}
          source={source(over)}
        />
      </TooltipProvider>,
    )
  })
  return { onOpenChange }
}

const dialog = () =>
  document.body.querySelector('[data-autocompact-dialog]') as HTMLElement
const tokensField = () =>
  dialog().querySelector('[name="autocompact-tokens"]') as HTMLInputElement
const summary = () =>
  dialog().querySelector(
    '[name="autocompact-instructions"]',
  ) as HTMLTextAreaElement
const button = (text: string) =>
  [...dialog().querySelectorAll('button')].find(
    (b) => b.textContent?.trim() === text,
  ) as HTMLButtonElement | undefined
const alerts = () =>
  [...dialog().querySelectorAll('[role="alert"]')].map((a) => a.textContent)

function type(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const proto =
    el instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set
  act(() => {
    setter?.call(el, value)
    el.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

async function enter(el: HTMLElement) {
  await act(async () => {
    el.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }),
    )
  })
}

/** The last patch written through `harness::chat-settings::set`. */
function lastPatch(): unknown {
  const calls = mocks.call.mock.calls.filter(
    ([fn]) => fn === 'harness::chat-settings::set',
  )
  return (calls.at(-1)?.[1] as { patch?: unknown } | undefined)?.patch
}

/** The handler error the engine wraps a typed rejection in. */
function rejection(code: string, message: string) {
  return {
    code: 'invocation_failed',
    message: `handler error: ${JSON.stringify({ code, message })}`,
  }
}

beforeEach(() => {
  setChatCapabilitiesForTests({
    autocompact: true,
    chatSettings: true,
    projects: false,
  })
  seedChatSettingsForTests({
    descriptions: { agent_writes: true },
    autocompact: GLOBAL,
  })
  seedChatProjectsForTests(PROJECTS)
  // A fake harness's chat settings: `get` answers what is stored, `set`
  // merges a patch per key, as `harness::chat-settings::set` does.
  let stored: Record<string, unknown> = {
    descriptions: { agent_writes: true },
    autocompact: GLOBAL,
  }
  mocks.call.mockReset()
  mocks.call.mockImplementation(
    async (fn: string, input?: { patch?: Record<string, unknown> }) => {
      if (fn === 'harness::chat-settings::set') {
        stored = { ...stored, ...input?.patch }
      }
      return { settings: stored }
    },
  )
  mocks.openConversation.mockReset()
  mocks.saveDetails.mockReset()
  mocks.conversations = [
    {
      id: 's-ledger',
      title: 'Ledger reconciliation',
      projectId: 'prj_billing',
      sessionMetadata: { autocompact: { mode: 'tokens', tokens: 600_000 } },
      messages: [],
      createdAt: 0,
      updatedAt: 0,
    },
    {
      id: 's-plain',
      title: 'Plain',
      sessionMetadata: {},
      messages: [],
      createdAt: 0,
      updatedAt: 0,
    },
    {
      id: 's-side',
      title: 'Side chat',
      sessionMetadata: {
        side_chat_of: 's-ledger',
        autocompact: { mode: 'off' },
      },
      messages: [],
      createdAt: 0,
      updatedAt: 0,
    },
  ]
})

afterEach(() => {
  act(() => root?.unmount())
  host?.remove()
  root = null
  host = null
  setSessionListHasMoreForTests(false)
  seedChatSettingsForTests(null)
  seedChatProjectsForTests(null)
  setChatCapabilitiesForTests(null)
})

describe('Settings › Chat › Auto-compact (§3.3.1)', () => {
  it("has the spec's copy", async () => {
    await render()
    const text = dialog().textContent ?? ''
    expect(text).toContain('Auto-compact')
    expect(text).toContain(
      "When a conversation's context passes this size at the end of a turn, the harness compacts it, as /compact does, and tells every device. Applies to every provider on this engine.",
    )
    expect(text).toContain('Compact conversations by themselves')
    expect(text).toContain('Compact at')
    expect(tokensField().value).toBe('400k')
    expect(text).toContain(
      "Whichever comes first: this size or the model's own limit. A model's limit is its input budget, less than its window: about 116k on a 200k Claude model. The grey ticks are the limits of the models used here in the last 7 days.",
    )
    expect(text).toContain('Models used here in the last 7 days')
    expect(text).toContain('What the summary keeps (optional)')
    expect(summary().placeholder).toBe(
      'Keep file paths, open TODOs and the PR numbers.',
    )
    expect(summary().value).toBe('Keep the PRs.')
    expect(text).toContain(
      'Sent with the compaction, like the text after /compact.',
    )
    expect(text).toContain('Projects and sessions')
    expect(text).toContain(
      "A project's value wins over the global; a session's over its project's.",
    )
    expect(text).toContain('Last auto-compactions')
  })

  it("lists each model's window, budget and outcome, with a tick at its budget", async () => {
    await render()
    const rows = [...dialog().querySelectorAll('[data-autocompact-model]')]
    expect(rows.map((r) => r.textContent)).toEqual([
      'Opus 5.5200k window · 116k input budgetAt its own limit, as today',
      'Sonnet 5.51M window · 916k input budgetAt 400k',
    ])
    expect(
      [...dialog().querySelectorAll('[data-slider-tick]')].map((t) =>
        t.getAttribute('data-slider-tick'),
      ),
    ).toEqual(['116000', '916000'])
  })

  it('says so when no model ran here lately', async () => {
    await render({ models: async () => [] })
    expect(dialog().textContent).toContain(
      'No model has run a turn on this engine in the last 7 days.',
    )
    expect(dialog().querySelector('[data-slider-tick]')).toBeNull()
  })

  it('writes a typed size at once, the whole setting through chat-settings', async () => {
    await render()
    type(tokensField(), '160k')
    await enter(tokensField())
    expect(lastPatch()).toEqual({
      autocompact: {
        mode: 'tokens',
        tokens: 160_000,
        instructions: 'Keep the PRs.',
      },
    })
    expect(tokensField().value).toBe('160k')
  })

  it('refuses a size out of range under the field, writing nothing', async () => {
    await render()
    type(tokensField(), '3M')
    await enter(tokensField())
    expect(lastPatch()).toBeUndefined()
    expect(alerts()).toEqual(['Between 20k and 2M tokens'])
  })

  it('puts the saved size back when the switch moves past text that is not a size', async () => {
    await render()
    const toggle = dialog().querySelector('[role="switch"]') as HTMLElement
    type(tokensField(), 'abc')
    await enter(tokensField())
    expect(alerts()).toEqual(['Between 20k and 2M tokens'])
    expect(lastPatch()).toBeUndefined()
    await act(async () => toggle.click())
    // Off keeps the saved size, and the field shows it: never "abc".
    expect(lastPatch()).toEqual({
      autocompact: {
        mode: 'off',
        tokens: 400_000,
        instructions: 'Keep the PRs.',
      },
    })
    expect(tokensField().value).toBe('400k')
    expect(alerts()).toEqual([])
    await act(async () => toggle.click())
    expect(lastPatch()).toEqual({
      autocompact: {
        mode: 'tokens',
        tokens: 400_000,
        instructions: 'Keep the PRs.',
      },
    })
    expect(tokensField().value).toBe('400k')
    expect(alerts()).toEqual([])
  })

  it('turns on at the size typed while it was on, and that size shows', async () => {
    await render()
    const toggle = dialog().querySelector('[role="switch"]') as HTMLElement
    type(tokensField(), '1,5M')
    await enter(tokensField())
    expect(lastPatch()).toEqual({
      autocompact: {
        mode: 'tokens',
        tokens: 1_500_000,
        instructions: 'Keep the PRs.',
      },
    })
    expect(tokensField().value).toBe('1.5M')
    await act(async () => toggle.click())
    expect(lastPatch()).toEqual({
      autocompact: {
        mode: 'off',
        tokens: 1_500_000,
        instructions: 'Keep the PRs.',
      },
    })
    expect(tokensField().value).toBe('1.5M')
  })

  it("shows the harness's bad_tokens under the field, keeping the typed size", async () => {
    mocks.call.mockRejectedValueOnce(
      rejection('bad_tokens', 'Between 20k and 2M tokens'),
    )
    await render()
    type(tokensField(), '160k')
    await enter(tokensField())
    expect(alerts()).toEqual(['Between 20k and 2M tokens'])
    expect(tokensField().value).toBe('160k')
    expect(document.activeElement).toBe(tokensField())
  })

  it('turns off, keeping the size for the next time it is on', async () => {
    await render()
    const toggle = dialog().querySelector('[role="switch"]') as HTMLElement
    expect(toggle.getAttribute('aria-checked')).toBe('true')
    await act(async () => toggle.click())
    expect(lastPatch()).toEqual({
      autocompact: {
        mode: 'off',
        tokens: 400_000,
        instructions: 'Keep the PRs.',
      },
    })
    expect(toggle.getAttribute('aria-checked')).toBe('false')
    expect(tokensField().disabled).toBe(true)
    // Off: every model compacts at its own limit.
    expect(
      [...dialog().querySelectorAll('[data-autocompact-outcome]')].map(
        (o) => o.textContent,
      ),
    ).toEqual(['At its own limit, as today', 'At its own limit, as today'])
    await act(async () => toggle.click())
    expect(lastPatch()).toEqual({
      autocompact: {
        mode: 'tokens',
        tokens: 400_000,
        instructions: 'Keep the PRs.',
      },
    })
  })

  it('saves the summary instructions, and shows bad_instructions under them', async () => {
    await render()
    expect(button('Save')).toBeUndefined()
    type(summary(), '  Keep the migration plan.  ')
    expect(dialog().textContent).toContain('28/1000')
    await act(async () => button('Save')?.click())
    expect(lastPatch()).toEqual({
      autocompact: {
        mode: 'tokens',
        tokens: 400_000,
        instructions: 'Keep the migration plan.',
      },
    })
    mocks.call.mockRejectedValueOnce(
      rejection('bad_instructions', 'Up to 1,000 characters'),
    )
    type(summary(), 'Something else')
    await act(async () => button('Save')?.click())
    expect(alerts()).toEqual(['Up to 1,000 characters'])
    // The refused text stays, to fix and save again.
    expect(summary().value).toBe('Something else')
    expect(button('Save')).toBeInstanceOf(HTMLButtonElement)
    expect(document.activeElement).toBe(summary())
    expect(
      summary().getAttribute('aria-describedby')?.endsWith('summary-error'),
    ).toBe(true)
  })

  it('clears the instructions with an empty string', async () => {
    await render()
    type(summary(), '')
    await act(async () => button('Save')?.click())
    expect(lastPatch()).toEqual({
      autocompact: { mode: 'tokens', tokens: 400_000, instructions: '' },
    })
  })

  it('shows any other failure at the top', async () => {
    mocks.call.mockRejectedValueOnce(new Error('engine down'))
    await render()
    type(tokensField(), '160k')
    await enter(tokensField())
    expect(alerts()).toEqual(['Not saved: engine down'])
  })
})

describe('the overrides table (D173)', () => {
  /** Where, Limit, Effective of each row, by its key. */
  function table(): Record<string, string[]> {
    return Object.fromEntries(
      [...dialog().querySelectorAll('tr[data-autocompact-override]')].map(
        (row) => [
          row.getAttribute('data-autocompact-override'),
          [...row.querySelectorAll('td')].map((td) => td.textContent ?? ''),
        ],
      ),
    )
  }

  it('lists every project and the sessions with a value of their own', async () => {
    await render()
    expect(table()).toEqual({
      'project:prj_web': [
        'acme-web',
        '160k',
        '160k on Sonnet 5.5 · Opus 5.5 at its own limit',
      ],
      'project:prj_billing': [
        'billing-api',
        'Global (400k)',
        '400k on Sonnet 5.5 · Opus 5.5 at its own limit',
      ],
      'project:prj_mobile': ['mobile', 'Off', "The model's limit only"],
      'session:s-ledger': [
        'Ledger reconciliation · session in billing-api',
        '600k',
        '600k on Sonnet 5.5 · Opus 5.5 at its own limit',
      ],
    })
    expect(
      dialog().querySelector('[data-autocompact-sessions]')?.textContent,
    ).toBe('1 session sets their own value')
  })

  it('reads "N+" while the session list has more pages', async () => {
    setSessionListHasMoreForTests(true)
    await render()
    expect(
      dialog().querySelector('[data-autocompact-sessions]')?.textContent,
    ).toBe('1+ sessions set their own value')
  })

  it("opens a project's dialog on its Auto-compact section", async () => {
    await render()
    const row = dialog().querySelector(
      'tr[data-autocompact-override="project:prj_web"]',
    ) as HTMLElement
    await act(async () => row.click())
    expect(
      document.body.querySelector('[data-project-autocompact]'),
    ).not.toBeNull()
  })

  it("opens a session's details on its Auto-compact row", async () => {
    await render()
    const row = dialog().querySelector(
      'tr[data-autocompact-override="session:s-ledger"]',
    ) as HTMLElement
    await act(async () => row.click())
    expect(
      document.body.querySelector('[data-conversation-autocompact]'),
    ).not.toBeNull()
  })
})

describe('the last auto-compactions', () => {
  it('lists them, and a tap opens the conversation', async () => {
    const { onOpenChange } = await render()
    const entry = dialog().querySelector(
      '[data-autocompact-log="s-ledger"]',
    ) as HTMLButtonElement
    expect(entry.textContent).toBe('Ledger reconciliation601k → 40k5m')
    await act(async () => entry.click())
    expect(mocks.openConversation).toHaveBeenCalledWith('s-ledger')
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it('says so when there are none, and shows a failed read', async () => {
    await render({
      log: async () => [],
      models: async () => {
        throw new Error('harness::autocompact::models timed out')
      },
    })
    expect(dialog().textContent).toContain('No auto-compactions yet.')
    expect(alerts()).toContain('harness::autocompact::models timed out')
  })
})

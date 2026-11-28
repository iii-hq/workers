// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { seedChatSettingsForTests } from '@/hooks/use-chat-settings'
import type { ChatProject } from '@/lib/backend/projects'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import {
  buildDetailsPatch,
  type ConversationDetailsDraft,
} from '@/lib/conversation-details'
import type { Conversation } from '@/types/chat'
import { ConversationDetailsDialog } from './ConversationDetailsDialog'
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

// No engine here: every call fails, so the seeded stores stand (a test
// answers `session::get` for a side chat's conversation).
const mocks = vi.hoisted(() => ({
  call: vi.fn<(fn: string, payload?: unknown) => Promise<unknown>>(),
}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: mocks.call }),
}))

const projects: ChatProject[] = [
  {
    id: 'prj_web',
    name: 'acme-web',
    color: 'blue',
    folders: ['/w'],
    created_at_ms: 0,
    updated_at_ms: 0,
    autocompact: { mode: 'tokens', tokens: 160_000 },
  },
  {
    id: 'prj_api',
    name: 'billing-api',
    color: 'teal',
    folders: ['/a'],
    created_at_ms: 0,
    updated_at_ms: 0,
  },
]

const base = {
  id: 'c1',
  title: 'Webhook retries',
  projectId: 'prj_web',
  sessionMetadata: {} as Record<string, unknown>,
}

let root: Root | null = null
let host: HTMLElement | null = null

function render(
  props: Partial<Parameters<typeof ConversationDetailsDialog>[0]> = {},
) {
  const onSave = vi.fn<(draft: ConversationDetailsDraft) => Promise<void>>(() =>
    Promise.resolve(),
  )
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <TooltipProvider delayDuration={0}>
        <ConversationDetailsDialog
          open
          conversation={base}
          onOpenChange={vi.fn()}
          onSave={onSave}
          projects={projects}
          {...props}
        />
      </TooltipProvider>,
    )
  })
  return { onSave }
}

const dialog = () =>
  document.body.querySelector('[role="dialog"]') as HTMLElement
const row = () =>
  dialog().querySelector(
    '[data-conversation-autocompact]',
  ) as HTMLElement | null
const trigger = () => row()?.querySelector('button') as HTMLButtonElement | null
const tokensField = () =>
  dialog().querySelector(
    '[name="conversation-autocompact-tokens"]',
  ) as HTMLInputElement | null
const save = () =>
  [...dialog().querySelectorAll('button')].find(
    (b) => b.textContent?.trim() === 'Save',
  ) as HTMLButtonElement

function type(el: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value',
  )?.set
  act(() => {
    setter?.call(el, value)
    el.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

/** The dialog focuses its first field a frame after it opens. */
/** Frames go by until `ready` holds (the dialog's first focus lands). */
async function settle(ready: () => boolean) {
  await act(async () => {
    for (let i = 0; i < 100 && !ready(); i++)
      await new Promise((resolve) => requestAnimationFrame(resolve))
  })
}

async function submit() {
  await act(async () => {
    save().click()
  })
}

beforeEach(() => {
  mocks.call.mockReset().mockRejectedValue(new Error('no engine in tests'))
  setChatCapabilitiesForTests({ autocompact: true, chatSettings: true })
  seedChatSettingsForTests({
    descriptions: { agent_writes: true },
    autocompact: { mode: 'tokens', tokens: 400_000 },
  })
})

afterEach(() => {
  act(() => root?.unmount())
  host?.remove()
  root = null
  setChatCapabilitiesForTests(null)
  seedChatSettingsForTests(null)
})

describe('Conversation details: Auto-compact (§3.3.4)', () => {
  it("offers the project's value first", () => {
    render()
    expect(row()?.textContent).toContain('Auto-compact')
    expect(trigger()?.textContent).toMatch(/^Project's \(160k\)\W*$/)
    expect(tokensField()).toBeNull()
  })

  it('offers the global for a conversation without a project', () => {
    render({ conversation: { ...base, projectId: undefined } })
    expect(trigger()?.textContent).toMatch(/^Global \(400k\)\W*$/)
  })

  it("follows the project picked in the dialog's draft", () => {
    render({ conversation: { ...base, projectId: 'prj_api' } })
    // billing-api follows the global.
    expect(trigger()?.textContent).toMatch(/^Project's \(400k\)\W*$/)
  })

  it('shows its own size in the field and saves a new one', async () => {
    const { onSave } = render({
      conversation: {
        ...base,
        sessionMetadata: { autocompact: { mode: 'tokens', tokens: 600_000 } },
      },
    })
    expect(trigger()?.textContent).toMatch(/^Compact at…\W*$/)
    expect(tokensField()?.value).toBe('600k')
    expect(dialog().textContent).toContain(
      "Whichever comes first: this size or the model's own limit.",
    )
    type(tokensField() as HTMLInputElement, '1.2M')
    await submit()
    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({
        autocompact: { mode: 'tokens', tokens: 1_200_000 },
      }),
    )
  })

  it('refuses a size out of range, under the field', async () => {
    const { onSave } = render({
      conversation: {
        ...base,
        sessionMetadata: { autocompact: { mode: 'tokens', tokens: 600_000 } },
      },
    })
    const name = dialog().querySelector('[name="conversation-title"]')
    await settle(() => document.activeElement === name)
    expect(document.activeElement).toBe(name)
    type(tokensField() as HTMLInputElement, '5k')
    await submit()
    expect(onSave).not.toHaveBeenCalled()
    expect(row()?.querySelector('[role="alert"]')?.textContent).toBe(
      'Between 20k and 2M tokens',
    )
    expect(document.activeElement).toBe(tokensField())
  })

  it("reads off as the model's limit", () => {
    render({
      conversation: {
        ...base,
        sessionMetadata: { autocompact: { mode: 'off' } },
      },
    })
    expect(trigger()?.textContent).toMatch(/^Only at the model's limit\W*$/)
  })

  it('saves the unchanged value along, which the patch then leaves out', async () => {
    const { onSave } = render()
    await submit()
    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ autocompact: { mode: 'inherit' } }),
    )
  })

  it('is a line in a side chat, which follows its conversation (D164)', async () => {
    const parent: Conversation = {
      id: 'root',
      title: 'Root',
      projectId: 'prj_api',
      model: null,
      messages: [],
      createdAt: 0,
      updatedAt: 0,
      sessionMetadata: { autocompact: { mode: 'tokens', tokens: 600_000 } },
    }
    const { onSave } = render({
      conversation: {
        ...base,
        sessionMetadata: { side_chat_of: 'root' },
      },
      conversations: [parent],
    })
    expect(trigger()).toBeNull()
    expect(row()?.querySelector('[data-autocompact-locked]')?.textContent).toBe(
      'Compacts at 600k, as its conversation does.',
    )
    await submit()
    expect(onSave.mock.calls[0]?.[0]).not.toHaveProperty('autocompact')
  })

  it("a side chat whose conversation is at the model's limit says so", () => {
    const parent: Conversation = {
      id: 'root',
      title: 'Root',
      model: null,
      messages: [],
      createdAt: 0,
      updatedAt: 0,
      sessionMetadata: {},
    }
    render({
      conversation: {
        ...base,
        projectId: undefined,
        sessionMetadata: { side_chat_of: 'root' },
      },
      conversations: [parent],
      sideChat: true,
    })
    // The global turned off on another device: the line follows it live.
    act(() =>
      seedChatSettingsForTests({
        descriptions: { agent_writes: true },
        autocompact: { mode: 'off' },
      }),
    )
    expect(row()?.querySelector('[data-autocompact-locked]')?.textContent).toBe(
      "Compacts at the model's limit, as its conversation does.",
    )
  })

  it("reads a side chat's conversation from the engine when it isn't loaded", async () => {
    mocks.call.mockImplementation(async (fn, payload) => {
      if (
        fn === 'session::get' &&
        (payload as { session_id?: string }).session_id === 'root'
      ) {
        return {
          meta: {
            session_id: 'root',
            metadata: {
              project_id: 'prj_api',
              autocompact: { mode: 'tokens', tokens: 600_000 },
            },
          },
        }
      }
      throw new Error('no engine in tests')
    })
    render({
      conversation: {
        ...base,
        // The copy frozen at the fork: never what the line says.
        sessionMetadata: {
          side_chat_of: 'root',
          autocompact: { mode: 'off' },
        },
      },
      conversations: [],
    })
    const line = () =>
      row()?.querySelector('[data-autocompact-locked]')?.textContent
    await settle(() => line() !== 'Compacts as its conversation does.')
    expect(line()).toBe('Compacts at 600k, as its conversation does.')
    expect(mocks.call).toHaveBeenCalledWith(
      'session::get',
      { session_id: 'root' },
      expect.anything(),
    )
  })

  it("says no size when a side chat's conversation can't be read", async () => {
    render({
      conversation: {
        ...base,
        sessionMetadata: {
          side_chat_of: 'gone',
          autocompact: { mode: 'tokens', tokens: 600_000 },
        },
      },
      conversations: [],
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(mocks.call).toHaveBeenCalledWith(
      'session::get',
      { session_id: 'gone' },
      expect.anything(),
    )
    expect(row()?.querySelector('[data-autocompact-locked]')?.textContent).toBe(
      'Compacts as its conversation does.',
    )
  })

  it.each([
    [123_456, '123k'],
    [400_500, '401k'],
    [1_234_567, '1.2M'],
  ])(
    'a rename keeps an exact size it did not edit (%d, shown %s)',
    async (exact, shown) => {
      const conversation = {
        ...base,
        sessionMetadata: { autocompact: { mode: 'tokens', tokens: exact } },
      }
      const { onSave } = render({ conversation })
      expect(tokensField()?.value).toBe(shown)
      type(
        dialog().querySelector(
          '[name="conversation-title"]',
        ) as HTMLInputElement,
        'Webhook retries, again',
      )
      await submit()
      const draft = onSave.mock.calls[0]?.[0] as ConversationDetailsDraft
      expect(draft.autocompact).toEqual({ mode: 'tokens', tokens: exact })
      // Only the name is written: no autocompact change.
      const patch = buildDetailsPatch(conversation, draft, 0)
      expect(patch?.title).toBe('Webhook retries, again')
      expect(patch?.set).not.toHaveProperty('autocompact')
      expect(patch?.unset).not.toContain('autocompact')
    },
  )

  it('a size picked again after another choice is still the exact one', async () => {
    const conversation = {
      ...base,
      sessionMetadata: { autocompact: { mode: 'tokens', tokens: 123_456 } },
    }
    const { onSave } = render({ conversation })
    type(tokensField() as HTMLInputElement, '124k')
    type(tokensField() as HTMLInputElement, '123k')
    await submit()
    expect(onSave.mock.calls[0]?.[0]).toEqual(
      expect.objectContaining({
        autocompact: { mode: 'tokens', tokens: 123_456 },
      }),
    )
  })

  it('names no inherited value before the settings are read', () => {
    seedChatSettingsForTests(null)
    render({ conversation: { ...base, projectId: undefined } })
    expect(trigger()?.textContent).toMatch(/^Global\W*$/)
    act(() => root?.unmount())
    host?.remove()
    // A project with its own value needs no global.
    render()
    expect(trigger()?.textContent).toMatch(/^Project's \(160k\)\W*$/)
    act(() => root?.unmount())
    host?.remove()
    render({ conversation: { ...base, projectId: 'prj_api' } })
    expect(trigger()?.textContent).toMatch(/^Project's\W*$/)
  })

  it("names no project's value before the project list answers", () => {
    setChatCapabilitiesForTests({
      autocompact: true,
      chatSettings: true,
      projects: true,
    })
    // The caller's list is empty until it loads.
    render({ projects: [] })
    expect(trigger()?.textContent).toMatch(/^Project's\W*$/)
  })

  it('a side chat says no size before the settings are read', () => {
    seedChatSettingsForTests(null)
    const parent: Conversation = {
      id: 'root',
      title: 'Root',
      model: null,
      messages: [],
      createdAt: 0,
      updatedAt: 0,
      sessionMetadata: {},
    }
    render({
      conversation: {
        ...base,
        projectId: undefined,
        sessionMetadata: { side_chat_of: 'root' },
      },
      conversations: [parent],
    })
    expect(row()?.querySelector('[data-autocompact-locked]')?.textContent).toBe(
      'Compacts as its conversation does.',
    )
  })

  it('opens on the row (the header chip)', async () => {
    render({ initialSection: 'autocompact' })
    await settle(() => document.activeElement === trigger())
    expect(document.activeElement).toBe(trigger())
  })

  it('is absent on a harness without auto-compact (D168)', () => {
    setChatCapabilitiesForTests({ autocompact: false, chatSettings: true })
    const { onSave } = render()
    expect(row()).toBeNull()
    act(() => save().click())
    expect(onSave.mock.calls[0]?.[0]).not.toHaveProperty('autocompact')
  })
})
